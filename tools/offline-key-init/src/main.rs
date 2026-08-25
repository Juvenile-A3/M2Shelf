#[cfg(not(windows))]
compile_error!("M2ShelfOfflineKeyInit is Windows-only because it requires CurrentUser DPAPI.");

use std::{
    env,
    ffi::{OsStr, OsString},
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::windows::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
    process::ExitCode,
    ptr::{null, null_mut},
    slice,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    },
    Storage::FileSystem::{
        GetDriveTypeW, GetVolumePathNameW, MoveFileExW, FILE_ATTRIBUTE_REPARSE_POINT,
        MOVEFILE_WRITE_THROUGH,
    },
    System::WindowsProgramming::DRIVE_FIXED,
};
use zeroize::{Zeroize, Zeroizing};

const CONFIRMATION: &str = "NEW_PRODUCTION_KEY";
const SEED_FILE: &str = "production-seed.dpapi";
const PUBLIC_KEY_FILE: &str = "update-public-key.txt";
const MAX_STAGING_ATTEMPTS: usize = 32;
const MAX_DPAPI_CIPHERTEXT_LEN: usize = 16 * 1024;

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|_| {
        eprintln!("M2ShelfOfflineKeyInit: internal failure; no secret data was printed.");
    }));
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("M2ShelfOfflineKeyInit: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), String> {
    if env::var_os("M2SHELF_UPDATE_PRIVATE_KEY").is_some() {
        return Err("refusing to run while M2SHELF_UPDATE_PRIVATE_KEY is set".into());
    }
    let output = parse_args(&args)?;
    let destination = validate_new_local_directory(&output)?;
    let parent = destination
        .parent()
        .ok_or_else(|| "output directory must have an existing parent".to_string())?;
    let mut staging = StagingDirectory::create(parent)?;

    let signing_key = SigningKey::generate(&mut OsRng);
    let mut seed = Zeroizing::new(signing_key.to_bytes());
    let public_key = signing_key.verifying_key().to_bytes();
    drop(signing_key);

    let protected = protect_current_user(&mut seed[..])?;
    let recovered = unprotect_current_user(&protected)?;
    let round_trip_matches = recovered.as_slice() == &seed[..];
    drop(recovered);
    if !round_trip_matches {
        return Err("CurrentUser DPAPI round-trip verification failed".into());
    }
    seed.zeroize();

    let public_key_text = format!("{}\n", BASE64_STANDARD.encode(public_key));
    let key_id = sha256_hex(&public_key);

    let seed_hash = write_new_synced_file(&staging.path, SEED_FILE, &protected)?;
    let public_hash =
        write_new_synced_file(&staging.path, PUBLIC_KEY_FILE, public_key_text.as_bytes())?;
    let report = build_report(&key_id, &seed_hash, &public_hash);
    commit_verified_directory(
        &mut staging,
        &destination,
        &protected,
        public_key_text.as_bytes(),
    )?;
    let _ = writeln!(std::io::stdout().lock(), "{report}");
    Ok(())
}

fn parse_args(args: &[OsString]) -> Result<PathBuf, String> {
    if args.len() != 5
        || args[0] != "init"
        || args[1] != "--output-directory"
        || args[3] != "--confirm"
        || args[4] != CONFIRMATION
    {
        return Err(format!(
            "usage: M2ShelfOfflineKeyInit.exe init --output-directory <absolute-new-local-directory> --confirm {CONFIRMATION}"
        ));
    }
    if args[2].is_empty() {
        return Err("output directory must not be empty".into());
    }
    Ok(PathBuf::from(&args[2]))
}

fn validate_new_local_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("output directory must be an absolute local path".into());
    }
    reject_unsafe_path_syntax(path)?;
    validate_destination_still_new(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| "output directory must have an existing parent".to_string())?;
    validate_existing_path_chain(parent)?;
    let parent = fs::canonicalize(parent)
        .map_err(|_| "output directory parent could not be resolved".to_string())?;
    ensure_plain_directory(&parent, "output directory parent")?;
    ensure_fixed_local_volume(&parent)?;
    reject_source_repository(&parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| "output directory must have a final directory name".to_string())?;
    let resolved = parent.join(file_name);
    validate_destination_still_new(&resolved)?;
    Ok(resolved)
}

fn ensure_fixed_local_volume(path: &Path) -> Result<(), String> {
    let path = to_wide(path.as_os_str());
    let mut volume_root = vec![0_u16; 32_768];
    let success = unsafe {
        GetVolumePathNameW(
            path.as_ptr(),
            volume_root.as_mut_ptr(),
            volume_root.len() as u32,
        )
    };
    if success == 0 {
        return Err("output-directory volume could not be resolved".into());
    }
    if unsafe { GetDriveTypeW(volume_root.as_ptr()) } != DRIVE_FIXED {
        return Err("output directory must be on a local fixed volume".into());
    }
    Ok(())
}

fn reject_source_repository(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        let git_marker = ancestor.join(".git");
        let alternate_git_marker = ancestor.join(".git-codex-local");
        let product_tree = ancestor.join("package.json").is_file()
            && ancestor.join("src-tauri").join("tauri.conf.json").is_file();
        if git_marker.exists() || alternate_git_marker.exists() || product_tree {
            return Err("output directory must be outside every source repository".into());
        }
    }
    Ok(())
}

fn reject_unsafe_path_syntax(path: &Path) -> Result<(), String> {
    let text = path.as_os_str().to_string_lossy();
    if text.starts_with("\\\\") || text.starts_with("//") {
        return Err("UNC and device paths are not allowed".into());
    }
    for component in path.components() {
        match component {
            Component::CurDir | Component::ParentDir => {
                return Err("relative path components are not allowed".into());
            }
            Component::Normal(value) => validate_windows_path_component(value)?,
            Component::Prefix(_) | Component::RootDir => {}
        }
    }
    Ok(())
}

fn validate_windows_path_component(value: &OsStr) -> Result<(), String> {
    let value = value
        .to_str()
        .ok_or_else(|| "output path must be valid Unicode".to_string())?;
    if value.contains(':') {
        return Err("alternate data stream syntax is not allowed".into());
    }
    if value.ends_with(' ') || value.ends_with('.') {
        return Err("output path components must not end with a space or period".into());
    }
    let device_stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .trim_end_matches([' ', '.']);
    if is_reserved_dos_device_name(device_stem) {
        return Err("reserved DOS device names are not allowed in the output path".into());
    }
    Ok(())
}

fn is_reserved_dos_device_name(value: &str) -> bool {
    matches!(
        value.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn validate_existing_path_chain(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| "every output-directory ancestor must already exist".to_string())?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("output-directory ancestors must not be reparse points".into());
        }
        if !metadata.is_dir() {
            return Err("output-directory ancestors must be directories".into());
        }
    }
    Ok(())
}

fn validate_destination_still_new(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err("output directory state could not be verified".into()),
        Ok(_) => Err("output directory already exists; refusing to overwrite".into()),
    }
}

fn ensure_plain_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{label} was not found"))?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!("{label} must be a plain directory"));
    }
    Ok(())
}

struct StagingDirectory {
    path: PathBuf,
    committed: bool,
}

impl StagingDirectory {
    fn create(parent: &Path) -> Result<Self, String> {
        ensure_plain_directory(parent, "output directory parent")?;
        for _ in 0..MAX_STAGING_ATTEMPTS {
            let candidate = parent.join(format!(
                ".m2shelf-offline-key-init-{:016x}",
                OsRng.next_u64()
            ));
            match fs::create_dir(&candidate) {
                Ok(()) => {
                    ensure_plain_directory(&candidate, "staging directory")?;
                    return Ok(Self {
                        path: candidate,
                        committed: false,
                    });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(_) => return Err("staging directory could not be created".into()),
            }
        }
        Err("a unique staging directory could not be created".into())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = cleanup_exact_staging(&self.path);
        }
    }
}

fn cleanup_exact_staging(path: &Path) -> Result<(), String> {
    validate_existing_path_chain(path)?;
    ensure_plain_directory(path, "staging directory")?;
    let mut removable = Vec::new();
    for entry in
        fs::read_dir(path).map_err(|_| "staging directory could not be enumerated".to_string())?
    {
        let entry = entry.map_err(|_| "staging directory could not be enumerated".to_string())?;
        let name = entry.file_name();
        if name != SEED_FILE && name != PUBLIC_KEY_FILE {
            return Err("staging directory contains an unexpected entry".into());
        }
        let candidate = entry.path();
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|_| "staging entry could not be verified".to_string())?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("staging entries must be plain files".into());
        }
        removable.push(candidate);
    }
    for candidate in removable {
        fs::remove_file(candidate).map_err(|_| "staging file cleanup failed".to_string())?;
    }
    fs::remove_dir(path).map_err(|_| "staging directory cleanup failed".to_string())
}

fn protect_current_user(secret: &mut [u8]) -> Result<Vec<u8>, String> {
    let ciphertext = crypt_data(secret, true)?;
    validate_dpapi_ciphertext(&ciphertext)?;
    Ok(ciphertext)
}

fn unprotect_current_user(ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    validate_dpapi_ciphertext(ciphertext)?;
    let mut owned = Zeroizing::new(ciphertext.to_vec());
    crypt_data(&mut owned, false).map(Zeroizing::new)
}

fn validate_dpapi_ciphertext(ciphertext: &[u8]) -> Result<(), String> {
    if ciphertext.is_empty() || ciphertext.len() > MAX_DPAPI_CIPHERTEXT_LEN {
        return Err("CurrentUser DPAPI ciphertext size is outside the accepted bound".into());
    }
    Ok(())
}

struct DpapiOutput {
    blob: CRYPT_INTEGER_BLOB,
    wipe_before_free: bool,
}

impl DpapiOutput {
    fn new(wipe_before_free: bool) -> Self {
        Self {
            blob: CRYPT_INTEGER_BLOB::default(),
            wipe_before_free,
        }
    }
}

impl Drop for DpapiOutput {
    fn drop(&mut self) {
        if self.blob.pbData.is_null() {
            return;
        }
        unsafe {
            if self.wipe_before_free && self.blob.cbData > 0 {
                slice::from_raw_parts_mut(self.blob.pbData, self.blob.cbData as usize).zeroize();
            }
            LocalFree(self.blob.pbData.cast());
        }
        self.blob = CRYPT_INTEGER_BLOB::default();
    }
}

fn crypt_data(input: &mut [u8], protect: bool) -> Result<Vec<u8>, String> {
    let input_len =
        u32::try_from(input.len()).map_err(|_| "DPAPI input is too large".to_string())?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: input.as_mut_ptr(),
    };
    let mut output = DpapiOutput::new(!protect);
    let success = unsafe {
        if protect {
            CryptProtectData(
                &input_blob,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.blob,
            )
        } else {
            CryptUnprotectData(
                &input_blob,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output.blob,
            )
        }
    };
    if success == 0 {
        return Err(format!(
            "CurrentUser DPAPI {} failed with Windows error {}",
            if protect {
                "protection"
            } else {
                "verification"
            },
            std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or_default()
        ));
    }
    if output.blob.pbData.is_null() || output.blob.cbData == 0 {
        return Err("CurrentUser DPAPI returned an empty result".into());
    }
    let result =
        unsafe { slice::from_raw_parts(output.blob.pbData, output.blob.cbData as usize).to_vec() };
    Ok(result)
}

fn write_new_synced_file(directory: &Path, name: &str, bytes: &[u8]) -> Result<String, String> {
    ensure_plain_directory(directory, "staging directory")?;
    let path = directory.join(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| format!("{name} could not be created"))?;
    file.write_all(bytes)
        .map_err(|_| format!("{name} could not be written"))?;
    file.sync_all()
        .map_err(|_| format!("{name} could not be synchronized"))?;
    drop(file);
    ensure_plain_file(&path, name, bytes.len())?;
    Ok(sha256_hex(bytes))
}

fn ensure_plain_file(path: &Path, label: &str, expected_len: usize) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("{label} was not found"))?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.len() != expected_len as u64
    {
        return Err(format!("{label} failed its post-write verification"));
    }
    Ok(())
}

fn verify_staging(directory: &Path, seed: &[u8], public_key: &[u8]) -> Result<(), String> {
    ensure_plain_directory(directory, "staging directory")?;
    ensure_exact_two_files(directory)?;
    verify_file_bytes(&directory.join(SEED_FILE), seed, SEED_FILE)?;
    verify_file_bytes(
        &directory.join(PUBLIC_KEY_FILE),
        public_key,
        PUBLIC_KEY_FILE,
    )
}

fn ensure_exact_two_files(directory: &Path) -> Result<(), String> {
    let mut names = fs::read_dir(directory)
        .map_err(|_| "key directory could not be enumerated".to_string())?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "key directory could not be enumerated".to_string())?;
    names.sort();
    let mut expected = vec![OsString::from(PUBLIC_KEY_FILE), OsString::from(SEED_FILE)];
    expected.sort();
    if names != expected {
        return Err("key directory contains an unexpected entry".into());
    }
    Ok(())
}

fn verify_file_bytes(path: &Path, expected: &[u8], label: &str) -> Result<(), String> {
    ensure_plain_file(path, label, expected.len())?;
    let actual = fs::read(path).map_err(|_| format!("{label} could not be read back"))?;
    if actual != expected {
        return Err(format!("{label} failed its content verification"));
    }
    Ok(())
}

fn validate_commit_preconditions(staging: &Path, destination: &Path) -> Result<(), String> {
    let destination_parent = destination
        .parent()
        .ok_or_else(|| "output directory must have an existing parent".to_string())?;
    validate_existing_path_chain(destination_parent)?;
    ensure_plain_directory(destination_parent, "output directory parent")?;
    ensure_fixed_local_volume(destination_parent)?;
    reject_source_repository(destination_parent)?;
    validate_destination_still_new(destination)?;

    validate_existing_path_chain(staging)?;
    ensure_plain_directory(staging, "staging directory")?;
    let canonical_parent = fs::canonicalize(destination_parent)
        .map_err(|_| "output directory parent could not be revalidated".to_string())?;
    let canonical_staging = fs::canonicalize(staging)
        .map_err(|_| "staging directory could not be revalidated".to_string())?;
    if canonical_staging.parent() != Some(canonical_parent.as_path()) {
        return Err("staging and output directories must retain the same plain parent".into());
    }
    Ok(())
}

fn verify_committed_directory(
    destination: &Path,
    seed: &[u8],
    public_key: &[u8],
) -> Result<(), String> {
    validate_existing_path_chain(destination)?;
    ensure_plain_directory(destination, "committed key directory")?;
    let parent = destination
        .parent()
        .ok_or_else(|| "committed key directory has no parent".to_string())?;
    ensure_fixed_local_volume(parent)?;
    reject_source_repository(parent)?;
    let canonical_parent = fs::canonicalize(parent)
        .map_err(|_| "committed key-directory parent could not be revalidated".to_string())?;
    let canonical_destination = fs::canonicalize(destination)
        .map_err(|_| "committed key directory could not be revalidated".to_string())?;
    if canonical_destination.parent() != Some(canonical_parent.as_path()) {
        return Err("committed key directory changed parents during verification".into());
    }
    verify_staging(destination, seed, public_key)
}

fn commit_verified_directory(
    staging: &mut StagingDirectory,
    destination: &Path,
    seed: &[u8],
    public_key: &[u8],
) -> Result<(), String> {
    validate_dpapi_ciphertext(seed)?;
    verify_staging(&staging.path, seed, public_key)?;
    validate_commit_preconditions(&staging.path, destination)?;
    move_directory_write_through(&staging.path, destination)?;
    staging.committed = true;
    if let Err(error) = verify_committed_directory(destination, seed, public_key) {
        return match cleanup_exact_staging(destination) {
            Ok(()) => Err(format!(
                "post-commit verification failed ({error}); the unreported output directory was removed"
            )),
            Err(cleanup_error) => Err(format!(
                "post-commit verification failed ({error}); automatic cleanup was refused ({cleanup_error}); inspect the new output directory manually"
            )),
        };
    }
    Ok(())
}

fn move_directory_write_through(source: &Path, destination: &Path) -> Result<(), String> {
    let source = to_wide(source.as_os_str());
    let destination = to_wide(destination.as_os_str());
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(format!(
            "atomic key-directory commit failed with Windows error {}",
            std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or_default()
        ));
    }
    Ok(())
}

fn to_wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn build_report(key_id: &str, seed_hash: &str, public_hash: &str) -> String {
    format!(
        "{{\"schemaVersion\":1,\"algorithm\":\"Ed25519\",\"protection\":\"CurrentUser-DPAPI\",\"keyId\":\"{key_id}\",\"seedFile\":\"{SEED_FILE}\",\"seedCiphertextSha256\":\"{seed_hash}\",\"publicKeyFile\":\"{PUBLIC_KEY_FILE}\",\"publicKeyFileSha256\":\"{public_hash}\"}}"
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{
        os::windows::fs::symlink_dir,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let parent = env::current_exe()
                .expect("test executable path")
                .parent()
                .expect("test executable parent")
                .to_path_buf();
            Self::new_in(&parent, label)
        }

        fn new_in(parent: &Path, label: &str) -> Self {
            for _ in 0..MAX_STAGING_ATTEMPTS {
                let counter = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
                let path = parent.join(format!(
                    ".m2shelf-offline-key-test-{label}-{}-{counter}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test directory could not be created: {error}"),
                }
            }
            panic!("unique test directory could not be created")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            for name in [
                SEED_FILE,
                PUBLIC_KEY_FILE,
                "existing.txt",
                "unexpected.txt",
                "ancestor-file",
            ] {
                let path = self.path.join(name);
                if fs::symlink_metadata(&path)
                    .map(|metadata| metadata.is_file())
                    .unwrap_or(false)
                {
                    let _ = fs::remove_file(path);
                }
            }
            let link = self.path.join("link");
            if fs::symlink_metadata(&link)
                .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
                .unwrap_or(false)
            {
                let _ = fs::remove_dir(link);
            }
            let target = self.path.join("target");
            let _ = fs::remove_dir(target);
            let _ = fs::remove_dir(&self.path);
        }
    }

    #[test]
    fn parser_requires_explicit_confirmation() {
        let valid = vec![
            OsString::from("init"),
            OsString::from("--output-directory"),
            OsString::from(r"C:\offline-key"),
            OsString::from("--confirm"),
            OsString::from(CONFIRMATION),
        ];
        assert_eq!(
            parse_args(&valid).expect("valid arguments"),
            PathBuf::from(r"C:\offline-key")
        );
        let mut invalid = valid;
        invalid[4] = OsString::from("YES");
        assert!(parse_args(&invalid).is_err());
    }

    #[test]
    fn path_syntax_rejects_ambiguous_windows_names() {
        for rejected in [
            r"\\server\share\key",
            r"\\?\C:\key",
            r"C:\keys\seed:stream",
            "C:\\keys\\trailing ",
            r"C:\keys\trailing.",
            r"C:\keys\CON",
            r"C:\keys\nul.txt",
            r"C:\keys\COM9.backup",
            r"C:\keys\LPT1",
            r"C:\keys\parent\..\key",
        ] {
            assert!(
                reject_unsafe_path_syntax(Path::new(rejected)).is_err(),
                "expected path rejection: {rejected}"
            );
        }
        for accepted in [r"C:\keys\safe", r"C:\keys\COM10", r"C:\keys\console.txt"] {
            assert!(
                reject_unsafe_path_syntax(Path::new(accepted)).is_ok(),
                "expected path acceptance: {accepted}"
            );
        }
    }

    #[test]
    fn dpapi_ciphertext_bound_matches_the_signing_contract() {
        assert!(validate_dpapi_ciphertext(&[]).is_err());
        assert!(validate_dpapi_ciphertext(&[0_u8; 1]).is_ok());
        let at_limit = vec![0_u8; MAX_DPAPI_CIPHERTEXT_LEN];
        assert!(validate_dpapi_ciphertext(&at_limit).is_ok());
        let over_limit = vec![0_u8; MAX_DPAPI_CIPHERTEXT_LEN + 1];
        assert!(validate_dpapi_ciphertext(&over_limit).is_err());
    }

    #[test]
    fn create_new_file_never_overwrites_existing_content() {
        let directory = TestDirectory::new("no-overwrite");
        let first = b"first non-production test payload";
        let second = b"replacement must be refused";
        write_new_synced_file(&directory.path, SEED_FILE, first).expect("first write");
        assert!(write_new_synced_file(&directory.path, SEED_FILE, second).is_err());
        assert_eq!(fs::read(directory.path.join(SEED_FILE)).unwrap(), first);
    }

    #[test]
    fn cleanup_removes_only_a_verified_staging_directory() {
        let directory = TestDirectory::new("cleanup-success");
        write_new_synced_file(&directory.path, SEED_FILE, b"test ciphertext").unwrap();
        write_new_synced_file(&directory.path, PUBLIC_KEY_FILE, b"test public key\n").unwrap();
        cleanup_exact_staging(&directory.path).expect("verified cleanup");
        assert!(!directory.path.exists());
    }

    #[test]
    fn cleanup_refuses_unexpected_entries_without_partial_deletion() {
        let directory = TestDirectory::new("cleanup-refusal");
        write_new_synced_file(&directory.path, SEED_FILE, b"test ciphertext").unwrap();
        fs::write(directory.path.join("unexpected.txt"), b"preserve").unwrap();
        assert!(cleanup_exact_staging(&directory.path).is_err());
        assert!(directory.path.join(SEED_FILE).is_file());
        assert!(directory.path.join("unexpected.txt").is_file());
    }

    #[test]
    fn existing_destination_is_always_rejected() {
        let directory = TestDirectory::new("existing-destination");
        assert!(validate_destination_still_new(&directory.path).is_err());
        let file = directory.path.join("existing.txt");
        fs::write(&file, b"preserve").unwrap();
        assert!(validate_destination_still_new(&file).is_err());
    }

    #[test]
    fn verified_commit_revalidates_and_moves_only_the_expected_dummy_files() {
        let directory = TestDirectory::new_in(&env::temp_dir(), "verified-commit");
        let staging_path = directory.path.join("staging");
        fs::create_dir(&staging_path).unwrap();
        let dummy_ciphertext = b"non-production DPAPI-shaped test bytes";
        let dummy_public_key = b"non-production public-key test bytes\n";
        write_new_synced_file(&staging_path, SEED_FILE, dummy_ciphertext).unwrap();
        write_new_synced_file(&staging_path, PUBLIC_KEY_FILE, dummy_public_key).unwrap();
        let mut staging = StagingDirectory {
            path: staging_path,
            committed: false,
        };
        let destination = directory.path.join("destination");

        commit_verified_directory(
            &mut staging,
            &destination,
            dummy_ciphertext,
            dummy_public_key,
        )
        .expect("verified dummy commit");

        assert!(staging.committed);
        verify_committed_directory(&destination, dummy_ciphertext, dummy_public_key).unwrap();
        cleanup_exact_staging(&destination).unwrap();
    }

    #[test]
    fn existing_path_chain_rejects_files_and_reparse_points() {
        let directory = TestDirectory::new("path-chain");
        let file = directory.path.join("ancestor-file");
        fs::write(&file, b"not a directory").unwrap();
        assert!(validate_existing_path_chain(&file.join("child")).is_err());

        let target = directory.path.join("target");
        let link = directory.path.join("link");
        fs::create_dir(&target).unwrap();
        if symlink_dir(&target, &link).is_ok() {
            assert!(validate_existing_path_chain(&link).is_err());
        }
    }

    #[test]
    fn report_contains_only_fixed_names_and_derived_values() {
        let report = build_report(&"a".repeat(64), &"b".repeat(64), &"c".repeat(64));
        assert!(report.contains("CurrentUser-DPAPI"));
        assert!(report.contains(SEED_FILE));
        assert!(report.contains(PUBLIC_KEY_FILE));
        assert!(!report.contains(r"C:\"));
        assert!(!report.contains("privateKey"));
        assert!(!report.contains("seedBase64"));
    }

    #[test]
    fn report_hashes_never_echo_input_bytes() {
        let secret_like = [0x5a_u8; 32];
        let hash = sha256_hex(&secret_like);
        assert_eq!(hash.len(), 64);
        assert!(!hash.contains("5a5a5a5a"));
    }
}
