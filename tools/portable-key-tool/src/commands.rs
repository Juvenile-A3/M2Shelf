use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, SecondsFormat, Utc};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use m2shelf_lib::update::{
    self, hash_file, sign_digest, verify_signature, APP_ID, NSIS_PLATFORM, PORTABLE_PLATFORM,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
    key_container::{
        self, decrypt_seed, encrypt_seed, KEY_DIRECTORY_NAME, KEY_FILE_NAME, METADATA_FILE_NAME,
        README_FILE_NAME,
    },
    password,
};

const MAX_KEY_FILE_BYTES: u64 = 1024;
const MAX_PUBLIC_KEY_FILE_BYTES: u64 = 4096;
const MAX_NOTES_FILE_BYTES: u64 = 256 * 1024;
const MAX_PROVENANCE_FILE_BYTES: u64 = 1024 * 1024;
const DEFAULT_KEY_ID: &str = "production-v0.5.11";
const RETURN_FILE_COUNT: usize = 8;

pub fn run(args: Vec<OsString>) -> Result<(), String> {
    if std::env::var_os("M2SHELF_UPDATE_PRIVATE_KEY").is_some() {
        return Err(
            "Refusing to run while the legacy signing-key environment variable is set.".into(),
        );
    }
    let (command, options) = parse_command(args)?;
    match command.as_str() {
        "migrate-dpapi" => migrate_dpapi(options),
        "verify-key" => verify_key(options),
        "sign-release" => sign_release(options),
        _ => Err(usage()),
    }
}

#[derive(Debug)]
struct MigrateArgs {
    input: PathBuf,
    output: PathBuf,
    public_key: PathBuf,
    key_id: String,
}

#[derive(Debug)]
struct VerifyArgs {
    key: PathBuf,
    public_key: PathBuf,
}

#[derive(Debug)]
struct SignReleaseArgs {
    key: PathBuf,
    public_key: PathBuf,
    version: String,
    candidate_directory: PathBuf,
    notes: PathBuf,
    provenance: PathBuf,
    output_directory: PathBuf,
    published_at: Option<String>,
}

fn migrate_dpapi(options: BTreeMap<String, OsString>) -> Result<(), String> {
    let args = MigrateArgs {
        input: required_path(&options, "input")?,
        output: required_path(&options, "output")?,
        public_key: required_path(&options, "public-key")?,
        key_id: optional_text(&options, "key-id")?.unwrap_or_else(|| DEFAULT_KEY_ID.to_owned()),
    };
    ensure_exact_options(&options, &["input", "output", "public-key"], &["key-id"])?;
    let password = password::prompt_new_password()?;
    migrate_dpapi_with_password(&args, &password)?;
    println!("Migration complete. The original DPAPI file was left unchanged.");
    Ok(())
}

fn verify_key(options: BTreeMap<String, OsString>) -> Result<(), String> {
    let args = VerifyArgs {
        key: required_path(&options, "key")?,
        public_key: required_path(&options, "public-key")?,
    };
    ensure_exact_options(&options, &["key", "public-key"], &[])?;
    let password = password::prompt_existing_password()?;
    verify_key_with_password(&args, &password)?;
    println!("USB signing key matches the current updater public key.");
    Ok(())
}

fn sign_release(options: BTreeMap<String, OsString>) -> Result<(), String> {
    let args = SignReleaseArgs {
        key: required_path(&options, "key")?,
        public_key: required_path(&options, "public-key")?,
        version: required_text(&options, "version")?,
        candidate_directory: required_path(&options, "candidate-directory")?,
        notes: required_path(&options, "notes")?,
        provenance: required_path(&options, "provenance")?,
        output_directory: required_path(&options, "output-directory")?,
        published_at: optional_text(&options, "published-at")?,
    };
    ensure_exact_options(
        &options,
        &[
            "key",
            "public-key",
            "version",
            "candidate-directory",
            "notes",
            "provenance",
            "output-directory",
        ],
        &["published-at"],
    )?;
    let password = password::prompt_existing_password()?;
    let output_zip = sign_release_with_password(&args, &password, true)?;
    println!("Signed release return package: {}", output_zip.display());
    println!("SHA-256 sidecar: {}.sha256", output_zip.display());
    Ok(())
}

fn migrate_dpapi_with_password(args: &MigrateArgs, password: &str) -> Result<(), String> {
    if args.output.file_name() != Some(OsStr::new(KEY_FILE_NAME)) {
        return Err(format!(
            "The migrated key file must be named {KEY_FILE_NAME}."
        ));
    }
    let parent = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            "The migrated key output must have an existing parent directory.".to_string()
        })?;
    if parent.file_name() != Some(OsStr::new(KEY_DIRECTORY_NAME)) {
        return Err(format!(
            "The migrated key must be stored inside {KEY_DIRECTORY_NAME}."
        ));
    }
    if !parent.is_dir() {
        return Err("The migrated key output parent directory does not exist.".into());
    }
    let expected_public_key = read_public_key(&args.public_key)?;
    let protected = Zeroizing::new(read_regular_bounded_file(
        &args.input,
        1,
        16 * 1024,
        "DPAPI seed file",
    )?);
    let plaintext = unprotect_current_user(&protected)?;
    let seed = Zeroizing::new(plaintext.as_slice().try_into().map_err(|_| {
        "The DPAPI file did not contain exactly one 32-byte Ed25519 seed.".to_string()
    })?);
    let old_signing_key = SigningKey::from_bytes(&seed);
    if old_signing_key.verifying_key() != expected_public_key {
        return Err("The DPAPI seed does not match the current updater public key.".into());
    }

    let (container, metadata) = encrypt_seed(&seed, password, &args.key_id)?;
    let migrated = decrypt_seed(&container, password)?;
    if migrated.public_key != expected_public_key.to_bytes()
        || migrated.seed.as_ref() != seed.as_ref()
    {
        return Err(
            "The migrated USB key failed its seed and public-key consistency check.".into(),
        );
    }
    let fixed_message = b"M2Shelf portable-key migration equivalence check v1";
    let old_signature = old_signing_key.sign(fixed_message);
    let migrated_signing_key = SigningKey::from_bytes(&migrated.seed);
    let migrated_signature = migrated_signing_key.sign(fixed_message);
    if old_signature != migrated_signature {
        return Err(
            "The migrated seed did not produce the same deterministic Ed25519 signature.".into(),
        );
    }
    drop(migrated_signing_key);
    drop(old_signing_key);
    drop(migrated);

    let metadata_path = parent.join(METADATA_FILE_NAME);
    let readme_path = parent.join(README_FILE_NAME);
    for path in [&args.output, &metadata_path, &readme_path] {
        if path.exists() {
            return Err(
                "The destination key directory already contains a managed key file.".into(),
            );
        }
    }

    let mut created = CreatedFiles::default();
    write_new_synced_file(&args.output, &container)?;
    created.track(args.output.clone());
    write_new_synced_file(&metadata_path, &key_container::metadata_json(&metadata)?)?;
    created.track(metadata_path);
    write_new_synced_file(&readme_path, key_container::readme_bytes())?;
    created.track(readme_path);

    let reread =
        read_regular_bounded_file(&args.output, 1, MAX_KEY_FILE_BYTES, "migrated USB key")?;
    let verified = decrypt_seed(&reread, password)?;
    if verified.public_key != expected_public_key.to_bytes()
        || verified.seed.as_ref() != seed.as_ref()
    {
        return Err("The migrated USB key failed its post-write verification.".into());
    }
    created.commit();
    Ok(())
}

fn verify_key_with_password(args: &VerifyArgs, password: &str) -> Result<(), String> {
    let expected_public_key = read_public_key(&args.public_key)?;
    let container =
        read_regular_bounded_file(&args.key, 1, MAX_KEY_FILE_BYTES, "encrypted USB key")?;
    let decrypted = decrypt_seed(&container, password)?;
    if decrypted.public_key != expected_public_key.to_bytes() {
        return Err("The USB signing key does not match the current updater public key.".into());
    }
    let signing_key = SigningKey::from_bytes(&decrypted.seed);
    if signing_key.verifying_key() != expected_public_key {
        return Err("The USB signing seed does not match the current updater public key.".into());
    }
    Ok(())
}

fn sign_release_with_password(
    args: &SignReleaseArgs,
    password: &str,
    require_embedded_key: bool,
) -> Result<PathBuf, String> {
    validate_canonical_version(&args.version)?;
    let expected_public_key = read_public_key(&args.public_key)?;
    if require_embedded_key {
        let identity = update::signer_identity_for_cli()?;
        if identity.app_id != APP_ID
            || identity.version != args.version
            || identity.public_key != BASE64_STANDARD.encode(expected_public_key.to_bytes())
        {
            return Err("This signing tool was not built from the requested version and updater public key.".into());
        }
    }

    let candidate_directory = regular_directory(&args.candidate_directory, "candidate directory")?;
    let output_directory = regular_directory(&args.output_directory, "output directory")?;
    let portable_name = format!("M2Shelf-Portable-{}-x64.zip", args.version);
    let nsis_name = format!("M2Shelf-Setup-{}-x64.exe", args.version);
    let portable_source = candidate_directory.join(&portable_name);
    let nsis_source = candidate_directory.join(&nsis_name);
    ensure_regular_bounded_file(
        &portable_source,
        1,
        update::MAX_ARTIFACT_BYTES,
        "Portable candidate",
    )?;
    ensure_regular_bounded_file(
        &nsis_source,
        1,
        update::MAX_ARTIFACT_BYTES,
        "NSIS candidate",
    )?;
    ensure_regular_bounded_file(
        &args.provenance,
        1,
        MAX_PROVENANCE_FILE_BYTES,
        "candidate provenance",
    )?;
    let notes = read_release_notes(&args.notes)?;
    let published_at = normalized_published_at(args.published_at.as_deref())?;

    let result_name = format!("M2Shelf-v{}-SIGNED-RETURN", args.version);
    let signed_directory = output_directory.join(&result_name);
    let output_zip = output_directory.join(format!("{result_name}.zip"));
    let output_hash = output_directory.join(format!("{result_name}.zip.sha256"));
    if signed_directory.exists() || output_zip.exists() || output_hash.exists() {
        return Err("A signed return output for this version already exists.".into());
    }
    fs::create_dir(&signed_directory)
        .map_err(|_| "Could not create the signed return output directory.".to_string())?;
    let mut outputs = CreatedOutput::new(
        signed_directory.clone(),
        output_zip.clone(),
        output_hash.clone(),
    );

    let portable_path = signed_directory.join(&portable_name);
    let nsis_path = signed_directory.join(&nsis_name);
    copy_new_file(&portable_source, &portable_path)?;
    copy_new_file(&nsis_source, &nsis_path)?;
    let provenance_path = signed_directory.join("candidate-provenance.json");
    copy_new_file(&args.provenance, &provenance_path)?;

    let container =
        read_regular_bounded_file(&args.key, 1, MAX_KEY_FILE_BYTES, "encrypted USB key")?;
    let decrypted = decrypt_seed(&container, password)?;
    if decrypted.public_key != expected_public_key.to_bytes() {
        return Err("The USB signing key does not match the current updater public key.".into());
    }
    let signing_key = SigningKey::from_bytes(&decrypted.seed);
    if signing_key.verifying_key() != expected_public_key {
        return Err("The USB signing seed does not match the current updater public key.".into());
    }
    let portable = sign_and_verify_asset(
        &portable_path,
        &args.version,
        PORTABLE_PLATFORM,
        &signing_key,
        &expected_public_key,
    )?;
    let nsis = sign_and_verify_asset(
        &nsis_path,
        &args.version,
        NSIS_PLATFORM,
        &signing_key,
        &expected_public_key,
    )?;
    drop(signing_key);
    drop(decrypted);

    write_new_synced_file(
        &signed_directory.join(format!("{portable_name}.sha256")),
        format!("{}  {portable_name}\r\n", portable.sha256).as_bytes(),
    )?;
    write_new_synced_file(
        &signed_directory.join(format!("{portable_name}.sig")),
        format!("{}\n", portable.signature).as_bytes(),
    )?;
    write_new_synced_file(
        &signed_directory.join(format!("{nsis_name}.sha256")),
        format!("{}  {nsis_name}\r\n", nsis.sha256).as_bytes(),
    )?;
    write_new_synced_file(
        &signed_directory.join(format!("{nsis_name}.sig")),
        format!("{}\n", nsis.signature).as_bytes(),
    )?;

    let manifest = UpdateManifest {
        schema_version: 1,
        version: args.version.clone(),
        published_at,
        notes,
        platforms: Platforms {
            portable: manifest_asset(&args.version, &portable, PORTABLE_PLATFORM),
            nsis: manifest_asset(&args.version, &nsis, NSIS_PLATFORM),
        },
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|_| "Could not serialize latest.json.".to_string())?;
    manifest_bytes.push(b'\n');
    write_new_synced_file(&signed_directory.join("latest.json"), &manifest_bytes)?;
    ensure_exact_signed_files(&signed_directory, &portable_name, &nsis_name)?;
    create_return_zip(&signed_directory, &output_zip, &portable_name, &nsis_name)?;
    outputs.mark_zip_created();
    let (zip_size, zip_digest) = hash_file(&output_zip)?;
    if zip_size == 0 {
        return Err("The signed return ZIP is empty.".into());
    }
    let zip_hash = update::encode_sha256(&zip_digest);
    let zip_file_name = output_zip
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "The signed return ZIP name is invalid.".to_string())?;
    write_new_synced_file(
        &output_hash,
        format!("{zip_hash}  {zip_file_name}\r\n").as_bytes(),
    )?;
    outputs.mark_hash_created();
    outputs.commit();
    Ok(output_zip)
}

#[derive(Debug, Clone)]
struct SignedAsset {
    file_name: String,
    size: u64,
    sha256: String,
    signature: String,
}

fn sign_and_verify_asset(
    path: &Path,
    version: &str,
    platform: &str,
    signing_key: &SigningKey,
    verifying_key: &VerifyingKey,
) -> Result<SignedAsset, String> {
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| "A release candidate has an invalid file name.".to_string())?
        .to_owned();
    let (size, digest) = hash_file(path)?;
    let signature = sign_digest(signing_key, version, platform, size, &digest)?;
    verify_signature(verifying_key, version, platform, size, &digest, &signature)?;
    Ok(SignedAsset {
        file_name,
        size,
        sha256: update::encode_sha256(&digest),
        signature: BASE64_STANDARD.encode(signature.to_bytes()),
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseNotes {
    #[serde(rename = "zh-CN")]
    zh_cn: String,
    #[serde(rename = "en-US")]
    en_us: String,
    #[serde(rename = "ja-JP")]
    ja_jp: String,
    #[serde(rename = "ko-KR")]
    ko_kr: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateManifest {
    schema_version: u32,
    version: String,
    published_at: String,
    notes: ReleaseNotes,
    platforms: Platforms,
}

#[derive(Serialize)]
struct Platforms {
    #[serde(rename = "windows-x64-portable")]
    portable: ManifestAsset,
    #[serde(rename = "windows-x64-nsis")]
    nsis: ManifestAsset,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestAsset {
    url: String,
    file_name: String,
    size: u64,
    sha256: String,
    signature: String,
}

fn manifest_asset(version: &str, asset: &SignedAsset, platform: &str) -> ManifestAsset {
    debug_assert!(matches!(platform, PORTABLE_PLATFORM | NSIS_PLATFORM));
    ManifestAsset {
        url: format!(
            "https://github.com/Undermori/M2Shelf/releases/download/v{version}/{}",
            asset.file_name
        ),
        file_name: asset.file_name.clone(),
        size: asset.size,
        sha256: asset.sha256.clone(),
        signature: asset.signature.clone(),
    }
}

fn read_release_notes(path: &Path) -> Result<ReleaseNotes, String> {
    let bytes = read_regular_bounded_file(path, 2, MAX_NOTES_FILE_BYTES, "release notes")?;
    let notes: ReleaseNotes = serde_json::from_slice(&bytes).map_err(|_| {
        "Release notes must be valid JSON with exactly four supported locales.".to_string()
    })?;
    for text in [&notes.zh_cn, &notes.en_us, &notes.ja_jp, &notes.ko_kr] {
        if text.trim().is_empty() || text.chars().count() > 50_000 {
            return Err("Every localized release note must be non-empty and bounded.".into());
        }
    }
    Ok(notes)
}

fn normalized_published_at(value: Option<&str>) -> Result<String, String> {
    let value = match value {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map_err(|_| "published-at must be a valid RFC 3339 timestamp.".to_string())?
            .with_timezone(&Utc),
        None => Utc::now(),
    };
    Ok(value.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn create_return_zip(
    signed_directory: &Path,
    output_zip: &Path,
    portable_name: &str,
    nsis_name: &str,
) -> Result<(), String> {
    let names = release_file_names(portable_name, nsis_name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_zip)
        .map_err(|_| "Could not create the signed return ZIP.".to_string())?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);
    let mut buffer = [0_u8; 64 * 1024];
    for name in names {
        writer
            .start_file(&name, options)
            .map_err(|_| "Could not add an entry to the signed return ZIP.".to_string())?;
        let mut input = File::open(signed_directory.join(&name))
            .map_err(|_| "Could not read a signed return file.".to_string())?;
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|_| "Could not read a signed return file.".to_string())?;
            if count == 0 {
                break;
            }
            writer
                .write_all(&buffer[..count])
                .map_err(|_| "Could not write the signed return ZIP.".to_string())?;
        }
    }
    let file = writer
        .finish()
        .map_err(|_| "Could not finalize the signed return ZIP.".to_string())?;
    file.sync_all()
        .map_err(|_| "Could not synchronize the signed return ZIP.".to_string())
}

fn ensure_exact_signed_files(
    directory: &Path,
    portable_name: &str,
    nsis_name: &str,
) -> Result<(), String> {
    let expected = release_file_names(portable_name, nsis_name)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let actual = fs::read_dir(directory)
        .map_err(|_| "Could not enumerate the signed return output.".to_string())?
        .map(|entry| {
            let entry =
                entry.map_err(|_| "Could not enumerate the signed return output.".to_string())?;
            let metadata = entry
                .metadata()
                .map_err(|_| "Could not inspect a signed return output.".to_string())?;
            if !metadata.is_file() {
                return Err("The signed return output must contain only regular files.".into());
            }
            entry
                .file_name()
                .into_string()
                .map_err(|_| "The signed return output has a non-Unicode file name.".to_string())
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    if actual.len() != RETURN_FILE_COUNT || actual != expected {
        return Err(
            "The signed return output does not contain the exact eight release files.".into(),
        );
    }
    Ok(())
}

fn release_file_names(portable_name: &str, nsis_name: &str) -> Vec<String> {
    vec![
        portable_name.to_owned(),
        format!("{portable_name}.sha256"),
        format!("{portable_name}.sig"),
        nsis_name.to_owned(),
        format!("{nsis_name}.sha256"),
        format!("{nsis_name}.sig"),
        "latest.json".to_owned(),
        "candidate-provenance.json".to_owned(),
    ]
}

fn read_public_key(path: &Path) -> Result<VerifyingKey, String> {
    let bytes =
        read_regular_bounded_file(path, 1, MAX_PUBLIC_KEY_FILE_BYTES, "updater public key")?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "The updater public key file is not valid UTF-8.".to_string())?
        .trim();
    let decoded = BASE64_STANDARD
        .decode(text)
        .map_err(|_| "The updater public key is not standard Base64.".to_string())?;
    if BASE64_STANDARD.encode(&decoded) != text {
        return Err("The updater public key is not canonical Base64.".into());
    }
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| "The updater public key must contain exactly 32 bytes.".to_string())?;
    VerifyingKey::from_bytes(&key).map_err(|_| "The updater public key is invalid.".to_string())
}

fn validate_canonical_version(version: &str) -> Result<(), String> {
    let parsed =
        Version::parse(version).map_err(|_| "The release version is invalid.".to_string())?;
    if parsed.pre.is_empty() && parsed.build.is_empty() && parsed.to_string() == version {
        Ok(())
    } else {
        Err(
            "The release version must be canonical major.minor.patch without prerelease metadata."
                .into(),
        )
    }
}

fn regular_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("The {label} was not found."))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!("The {label} must be a regular directory."));
    }
    fs::canonicalize(path).map_err(|_| format!("The {label} could not be resolved."))
}

fn ensure_regular_bounded_file(path: &Path, min: u64, max: u64, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| format!("The {label} was not found."))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || !(min..=max).contains(&metadata.len())
    {
        return Err(format!("The {label} is not a bounded regular file."));
    }
    Ok(())
}

fn read_regular_bounded_file(
    path: &Path,
    min: u64,
    max: u64,
    label: &str,
) -> Result<Vec<u8>, String> {
    ensure_regular_bounded_file(path, min, max, label)?;
    let bytes = fs::read(path).map_err(|_| format!("The {label} could not be read."))?;
    if !(min..=max).contains(&(bytes.len() as u64)) {
        return Err(format!("The {label} changed while it was being read."));
    }
    Ok(bytes)
}

fn copy_new_file(source: &Path, destination: &Path) -> Result<(), String> {
    let mut input =
        File::open(source).map_err(|_| "Could not open a release input.".to_string())?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|_| "Could not create a signed return file.".to_string())?;
    std::io::copy(&mut input, &mut output)
        .map_err(|_| "Could not copy a release input.".to_string())?;
    output
        .sync_all()
        .map_err(|_| "Could not synchronize a signed return file.".to_string())
}

fn write_new_synced_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Could not create an output file.".to_string())?;
    let result = file
        .write_all(bytes)
        .map_err(|_| "Could not write an output file.".to_string())
        .and_then(|()| {
            file.sync_all()
                .map_err(|_| "Could not synchronize an output file.".to_string())
        });
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

#[derive(Default)]
struct CreatedFiles {
    paths: Vec<PathBuf>,
    committed: bool,
}

impl CreatedFiles {
    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for CreatedFiles {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for path in self.paths.iter().rev() {
            let _ = fs::remove_file(path);
        }
    }
}

struct CreatedOutput {
    directory: PathBuf,
    zip: PathBuf,
    hash: PathBuf,
    zip_created: bool,
    hash_created: bool,
    committed: bool,
}

impl CreatedOutput {
    fn new(directory: PathBuf, zip: PathBuf, hash: PathBuf) -> Self {
        Self {
            directory,
            zip,
            hash,
            zip_created: false,
            hash_created: false,
            committed: false,
        }
    }

    fn mark_zip_created(&mut self) {
        self.zip_created = true;
    }

    fn mark_hash_created(&mut self) {
        self.hash_created = true;
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for CreatedOutput {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        if self.hash_created {
            let _ = fs::remove_file(&self.hash);
        }
        if self.zip_created || self.zip.exists() {
            let _ = fs::remove_file(&self.zip);
        }
        if self.directory.exists() {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}

fn parse_command(args: Vec<OsString>) -> Result<(String, BTreeMap<String, OsString>), String> {
    let mut args = args.into_iter();
    let command = args
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or_else(usage)?;
    let remaining = args.collect::<Vec<_>>();
    if remaining.len() % 2 != 0 {
        return Err(usage());
    }
    let mut options = BTreeMap::new();
    for pair in remaining.chunks_exact(2) {
        let flag = pair[0].to_str().ok_or_else(usage)?;
        let name = flag
            .strip_prefix("--")
            .filter(|name| !name.is_empty())
            .ok_or_else(usage)?;
        if options.insert(name.to_owned(), pair[1].clone()).is_some() {
            return Err(usage());
        }
    }
    Ok((command, options))
}

fn required_path(options: &BTreeMap<String, OsString>, name: &str) -> Result<PathBuf, String> {
    let value = options
        .get(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(usage)?;
    Ok(PathBuf::from(value))
}

fn required_text(options: &BTreeMap<String, OsString>, name: &str) -> Result<String, String> {
    optional_text(options, name)?.ok_or_else(usage)
}

fn optional_text(
    options: &BTreeMap<String, OsString>,
    name: &str,
) -> Result<Option<String>, String> {
    options
        .get(name)
        .map(|value| {
            value
                .to_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(usage)
        })
        .transpose()
}

fn ensure_exact_options(
    options: &BTreeMap<String, OsString>,
    required: &[&str],
    optional: &[&str],
) -> Result<(), String> {
    let expected = required
        .iter()
        .chain(optional.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    if options.keys().any(|key| !expected.contains(key.as_str()))
        || required.iter().any(|key| !options.contains_key(*key))
    {
        return Err(usage());
    }
    Ok(())
}

fn usage() -> String {
    "usage:\n  M2ShelfPortableKeyTool migrate-dpapi --input <production-seed.dpapi> --output <encrypted-private-key.m2key> --public-key <update-public-key.txt> [--key-id <id>]\n  M2ShelfPortableKeyTool verify-key --key <encrypted-private-key.m2key> --public-key <update-public-key.txt>\n  M2ShelfPortableKeyTool sign-release --key <encrypted-private-key.m2key> --public-key <update-public-key.txt> --version <major.minor.patch> --candidate-directory <dir> --notes <release-notes.json> --provenance <candidate-provenance.json> --output-directory <dir> [--published-at <RFC3339>]".into()
}

#[cfg(windows)]
fn unprotect_current_user(ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    use std::{ptr::null_mut, slice};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };

    struct Output {
        blob: CRYPT_INTEGER_BLOB,
    }
    impl Drop for Output {
        fn drop(&mut self) {
            if self.blob.pbData.is_null() {
                return;
            }
            unsafe {
                if self.blob.cbData > 0 {
                    slice::from_raw_parts_mut(self.blob.pbData, self.blob.cbData as usize)
                        .zeroize();
                }
                LocalFree(self.blob.pbData.cast());
            }
        }
    }

    let mut input = Zeroizing::new(ciphertext.to_vec());
    let input_len =
        u32::try_from(input.len()).map_err(|_| "The DPAPI seed file is too large.".to_string())?;
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: input.as_mut_ptr(),
    };
    let mut output = Output {
        blob: CRYPT_INTEGER_BLOB::default(),
    };
    let success = unsafe {
        CryptUnprotectData(
            &input_blob,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.blob,
        )
    };
    if success == 0 || output.blob.pbData.is_null() || output.blob.cbData == 0 {
        return Err(
            "CurrentUser DPAPI could not decrypt the legacy seed for this Windows user.".into(),
        );
    }
    Ok(Zeroizing::new(
        unsafe { slice::from_raw_parts(output.blob.pbData, output.blob.cbData as usize) }.to_vec(),
    ))
}

#[cfg(not(windows))]
fn unprotect_current_user(_ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    Err("DPAPI migration is available only on Windows.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const TEST_PASSWORD: &str = "correct horse battery staple";

    fn write(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
    }

    fn test_public_key(path: &Path, seed: &[u8; 32]) {
        let public = SigningKey::from_bytes(seed).verifying_key().to_bytes();
        write(
            path,
            format!("{}\n", BASE64_STANDARD.encode(public)).as_bytes(),
        );
    }

    #[test]
    fn verify_key_accepts_only_the_matching_public_key() {
        let temp = TempDir::new().unwrap();
        let seed = [11_u8; 32];
        let (container, _) = encrypt_seed(&seed, TEST_PASSWORD, "test-key").unwrap();
        let key_path = temp.path().join(KEY_FILE_NAME);
        let public_path = temp.path().join("public.txt");
        write(&key_path, &container);
        test_public_key(&public_path, &seed);
        verify_key_with_password(
            &VerifyArgs {
                key: key_path,
                public_key: public_path.clone(),
            },
            TEST_PASSWORD,
        )
        .unwrap();
        test_public_key(&public_path, &[12_u8; 32]);
        assert!(verify_key_with_password(
            &VerifyArgs {
                key: temp.path().join(KEY_FILE_NAME),
                public_key: public_path,
            },
            TEST_PASSWORD,
        )
        .is_err());
    }

    #[test]
    fn migration_requires_the_fixed_usb_key_directory() {
        let temp = TempDir::new().unwrap();
        let error = migrate_dpapi_with_password(
            &MigrateArgs {
                input: temp.path().join("missing.dpapi"),
                output: temp.path().join(KEY_FILE_NAME),
                public_key: temp.path().join("missing-public-key.txt"),
                key_id: "test-key".to_owned(),
            },
            TEST_PASSWORD,
        )
        .unwrap_err();
        assert!(error.contains(KEY_DIRECTORY_NAME));
    }

    #[test]
    fn test_seed_sign_release_produces_the_frozen_eight_file_contract() {
        let temp = TempDir::new().unwrap();
        let candidate = temp.path().join("candidate");
        let output = temp.path().join("output");
        fs::create_dir(&candidate).unwrap();
        fs::create_dir(&output).unwrap();
        let version = "1.2.3";
        write(
            &candidate.join(format!("M2Shelf-Portable-{version}-x64.zip")),
            b"portable test candidate",
        );
        write(
            &candidate.join(format!("M2Shelf-Setup-{version}-x64.exe")),
            b"nsis test candidate",
        );
        let provenance = candidate.join("candidate-provenance.json");
        write(&provenance, b"{\"test\":true}\n");
        let notes = candidate.join("release-notes.json");
        write(
            &notes,
            br#"{"zh-CN":"test","en-US":"test","ja-JP":"test","ko-KR":"test"}"#,
        );
        let seed = [21_u8; 32];
        let public_key = temp.path().join("public.txt");
        test_public_key(&public_key, &seed);
        let key = temp.path().join(KEY_FILE_NAME);
        write(
            &key,
            &encrypt_seed(&seed, TEST_PASSWORD, "test-key").unwrap().0,
        );
        let zip = sign_release_with_password(
            &SignReleaseArgs {
                key,
                public_key,
                version: version.to_owned(),
                candidate_directory: candidate,
                notes,
                provenance,
                output_directory: output.clone(),
                published_at: Some("2026-08-25T12:34:56Z".to_owned()),
            },
            TEST_PASSWORD,
            false,
        )
        .unwrap();
        assert!(zip.is_file());
        assert!(PathBuf::from(format!("{}.sha256", zip.display())).is_file());
        let signed = output.join(format!("M2Shelf-v{version}-SIGNED-RETURN"));
        ensure_exact_signed_files(
            &signed,
            &format!("M2Shelf-Portable-{version}-x64.zip"),
            &format!("M2Shelf-Setup-{version}-x64.exe"),
        )
        .unwrap();

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(signed.join("latest.json")).unwrap()).unwrap();
        assert_eq!(manifest["schemaVersion"], 1);
        assert_eq!(manifest["version"], version);
        assert_eq!(manifest["publishedAt"], "2026-08-25T12:34:56Z");
        for (platform, file_name) in [
            (
                PORTABLE_PLATFORM,
                format!("M2Shelf-Portable-{version}-x64.zip"),
            ),
            (NSIS_PLATFORM, format!("M2Shelf-Setup-{version}-x64.exe")),
        ] {
            let signature = manifest["platforms"][platform]["signature"]
                .as_str()
                .unwrap();
            let verified = update::verify_signature(
                &SigningKey::from_bytes(&seed).verifying_key(),
                version,
                platform,
                manifest["platforms"][platform]["size"].as_u64().unwrap(),
                &update::decode_sha256(manifest["platforms"][platform]["sha256"].as_str().unwrap())
                    .unwrap(),
                &update::decode_signature(signature).unwrap(),
            );
            assert!(verified.is_ok(), "{file_name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_migration_keeps_the_test_seed_and_original_file() {
        use std::ptr::null;
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::Cryptography::{
                CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
            },
        };

        let temp = TempDir::new().unwrap();
        let key_directory = temp.path().join("M2Shelf-Production-Key");
        fs::create_dir(&key_directory).unwrap();
        let seed = [31_u8; 32];
        let mut seed_input = seed;
        let input_blob = CRYPT_INTEGER_BLOB {
            cbData: seed_input.len() as u32,
            pbData: seed_input.as_mut_ptr(),
        };
        let mut output_blob = CRYPT_INTEGER_BLOB::default();
        let success = unsafe {
            CryptProtectData(
                &input_blob,
                null(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output_blob,
            )
        };
        seed_input.zeroize();
        assert_ne!(success, 0);
        let protected = unsafe {
            std::slice::from_raw_parts(output_blob.pbData, output_blob.cbData as usize).to_vec()
        };
        unsafe { LocalFree(output_blob.pbData.cast()) };
        let input = temp.path().join("legacy-test.dpapi");
        write(&input, &protected);
        let public_key = temp.path().join("public.txt");
        test_public_key(&public_key, &seed);
        let output = key_directory.join(KEY_FILE_NAME);
        migrate_dpapi_with_password(
            &MigrateArgs {
                input: input.clone(),
                output: output.clone(),
                public_key,
                key_id: "test-key".to_owned(),
            },
            TEST_PASSWORD,
        )
        .unwrap();
        assert!(input.is_file());
        assert!(output.is_file());
        assert!(key_directory.join(METADATA_FILE_NAME).is_file());
        assert!(key_directory.join(README_FILE_NAME).is_file());
        let recovered = decrypt_seed(&fs::read(output).unwrap(), TEST_PASSWORD).unwrap();
        assert_eq!(recovered.seed.as_ref(), &seed);
    }

    #[test]
    fn cli_parser_never_needs_to_render_option_values() {
        let parsed = parse_command(vec![
            OsString::from("verify-key"),
            OsString::from("--key"),
            OsString::from("X:\\private\\secret.m2key"),
            OsString::from("--public-key"),
            OsString::from("public.txt"),
        ])
        .unwrap();
        assert_eq!(parsed.0, "verify-key");
        assert!(!usage().contains("X:\\private"));
    }
}
