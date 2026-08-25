use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, TryLockError},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, SecondsFormat, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
    Url,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    cache,
    models::{
        AvailableUpdate, UpdateCheckResult, UpdateDistribution, UpdateDownloadStatus, UpdatePhase,
    },
};

pub const APP_ID: &str = "app.morimediashelf.desktop";
pub const UPDATE_ENDPOINT: &str =
    "https://github.com/Undermori/M2Shelf/releases/latest/download/latest.json";
pub const PORTABLE_PLATFORM: &str = "windows-x64-portable";
pub const NSIS_PLATFORM: &str = "windows-x64-nsis";
pub const PORTABLE_MARKER_FILE: &str = "M2Shelf.portable.json";
pub const UPDATER_FILE: &str = "M2ShelfUpdater.exe";

const SIGNATURE_DOMAIN: &[u8] = b"M2Shelf.Update.v1\0";
const PUBLIC_KEY_TEXT: &str = include_str!("../update-public-key.txt");
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RELEASE_NOTE_CHARS: usize = 50_000;
const MAX_VERSION_CHARS: usize = 80;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateManifest {
    schema_version: u32,
    version: String,
    published_at: String,
    notes: ReleaseNotes,
    platforms: Platforms,
}

#[derive(Debug, Clone, Deserialize)]
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

impl ReleaseNotes {
    fn validate(&self) -> Result<(), String> {
        for (locale, note) in [
            ("zh-CN", &self.zh_cn),
            ("en-US", &self.en_us),
            ("ja-JP", &self.ja_jp),
            ("ko-KR", &self.ko_kr),
        ] {
            if note.trim().is_empty() {
                return Err(format!("更新说明 {locale} 不能为空。"));
            }
            if note.chars().count() > MAX_RELEASE_NOTE_CHARS {
                return Err(format!("更新说明 {locale} 过长。"));
            }
        }
        Ok(())
    }

    fn into_map(self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("zh-CN".into(), self.zh_cn),
            ("en-US".into(), self.en_us),
            ("ja-JP".into(), self.ja_jp),
            ("ko-KR".into(), self.ko_kr),
        ])
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Platforms {
    #[serde(rename = "windows-x64-portable")]
    portable: UpdateAsset,
    #[serde(rename = "windows-x64-nsis")]
    nsis: UpdateAsset,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpdateAsset {
    pub(crate) url: String,
    pub(crate) file_name: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
    pub(crate) signature: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CheckedUpdate {
    pub(crate) distribution: UpdateDistribution,
    pub(crate) platform: &'static str,
    pub(crate) version: String,
    pub(crate) published_at: String,
    pub(crate) notes: BTreeMap<String, String>,
    pub(crate) asset: UpdateAsset,
}

#[derive(Debug, Clone)]
pub(crate) struct DownloadedUpdate {
    pub(crate) checked: CheckedUpdate,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Default)]
struct UpdateRuntime {
    checked: Option<CheckedUpdate>,
    downloaded: Option<DownloadedUpdate>,
    status: UpdateDownloadStatus,
}

#[derive(Debug, Clone)]
pub struct UpdateManager {
    cache_dir: PathBuf,
    runtime: Arc<Mutex<UpdateRuntime>>,
    operation: Arc<Mutex<()>>,
}

impl UpdateManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            runtime: Arc::new(Mutex::new(UpdateRuntime::default())),
            operation: Arc::new(Mutex::new(())),
        }
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub(crate) fn begin_operation(&self) -> Result<MutexGuard<'_, ()>, String> {
        match self.operation.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err("另一项更新操作正在进行，请稍候。".into()),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
        }
    }

    pub fn status(&self) -> UpdateDownloadStatus {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status
            .clone()
    }

    fn set_status(&self, status: UpdateDownloadStatus) {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status = status;
    }

    pub fn set_applying(&self, version: &str) {
        self.set_status(UpdateDownloadStatus {
            phase: UpdatePhase::Applying,
            version: Some(version.to_owned()),
            downloaded_bytes: 0,
            total_bytes: None,
            error: None,
            can_install: false,
        });
    }

    pub fn fail(&self, version: Option<&str>, error: &str) {
        self.set_status(UpdateDownloadStatus {
            phase: UpdatePhase::Failed,
            version: version.map(str::to_owned),
            downloaded_bytes: 0,
            total_bytes: None,
            error: Some(error.to_owned()),
            can_install: false,
        });
    }

    pub fn check(&self, distribution: UpdateDistribution) -> Result<UpdateCheckResult, String> {
        let _operation = self.begin_operation()?;
        self.set_status(UpdateDownloadStatus {
            phase: UpdatePhase::Checking,
            ..UpdateDownloadStatus::default()
        });

        let result: Result<UpdateCheckResult, String> = (|| -> Result<_, String> {
            // Fail before making a network request when release verification has not been
            // configured. A package is never trusted solely because it came from GitHub.
            configured_verifying_key()?;
            let client = http_client(REQUEST_TIMEOUT)?;
            let response = client
                .get(UPDATE_ENDPOINT)
                .header(reqwest::header::ACCEPT, "application/json")
                .send()
                .map_err(|error| format!("无法获取更新清单：{error}"))?;
            let checked = match read_update_manifest(response, env!("CARGO_PKG_VERSION"))? {
                Some(bytes) => validate_manifest(&bytes, env!("CARGO_PKG_VERSION"), distribution)?,
                None => None,
            };
            let checked_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
            let public = checked.as_ref().map(CheckedUpdate::to_public);
            {
                let mut runtime = self
                    .runtime
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                runtime.checked = checked;
                runtime.downloaded = None;
                runtime.status = UpdateDownloadStatus::default();
            }
            Ok(UpdateCheckResult {
                current_version: env!("CARGO_PKG_VERSION").into(),
                distribution,
                checked_at,
                update: public,
            })
        })();

        if let Err(error) = &result {
            self.fail(None, error);
        }
        result
    }

    pub fn download(
        &self,
        version: &str,
        library_roots: &[PathBuf],
    ) -> Result<UpdateDownloadStatus, String> {
        let _operation = self.begin_operation()?;
        let checked = {
            let runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            runtime
                .checked
                .as_ref()
                .filter(|checked| checked.version == version)
                .cloned()
                .ok_or_else(|| "请先检查更新，且只能下载刚刚验证过的版本。".to_string())?
        };
        ensure_newer_version(env!("CARGO_PKG_VERSION"), &checked.version)?;
        configured_verifying_key()?;

        self.set_status(UpdateDownloadStatus {
            phase: UpdatePhase::Downloading,
            version: Some(checked.version.clone()),
            downloaded_bytes: 0,
            total_bytes: Some(checked.asset.size),
            error: None,
            can_install: false,
        });

        let result = self.download_checked(checked.clone(), library_roots);
        match result {
            Ok(downloaded) => {
                let status = UpdateDownloadStatus {
                    phase: UpdatePhase::Ready,
                    version: Some(checked.version.clone()),
                    downloaded_bytes: checked.asset.size,
                    total_bytes: Some(checked.asset.size),
                    error: None,
                    can_install: true,
                };
                let mut runtime = self
                    .runtime
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                runtime.downloaded = Some(downloaded);
                runtime.status = status.clone();
                Ok(status)
            }
            Err(error) => {
                self.fail(Some(&checked.version), &error);
                Err(error)
            }
        }
    }

    fn download_checked(
        &self,
        checked: CheckedUpdate,
        library_roots: &[PathBuf],
    ) -> Result<DownloadedUpdate, String> {
        let version_dir = ensure_safe_update_subdirectory(
            &self.cache_dir,
            &[checked.version.as_str()],
            library_roots,
        )?;
        let destination = version_dir.join(&checked.asset.file_name);
        cleanup_owned_partial_downloads(&version_dir, &checked.asset.file_name);
        let partial = version_dir.join(format!(
            ".{}.{}.partial",
            checked.asset.file_name,
            Uuid::new_v4()
        ));
        let cleanup = PartialFileGuard(partial.clone());

        let client = http_client(DOWNLOAD_TIMEOUT)?;
        let response = client
            .get(&checked.asset.url)
            .header(reqwest::header::ACCEPT, "application/octet-stream")
            .send()
            .map_err(|error| format!("无法下载更新包：{error}"))?;
        ensure_success_response(&response, "更新包")?;
        if response
            .content_length()
            .is_some_and(|length| length != checked.asset.size)
        {
            return Err("更新包 Content-Length 与清单不一致。".into());
        }
        let mut response = response;
        validate_existing_update_subdirectory(
            &self.cache_dir,
            &[checked.version.as_str()],
            library_roots,
        )?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| format!("无法创建更新包临时文件：{error}"))?;
        let mut hasher = Sha256::new();
        let mut downloaded = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = response
                .read(&mut buffer)
                .map_err(|error| format!("读取更新包失败：{error}"))?;
            if count == 0 {
                break;
            }
            downloaded = downloaded
                .checked_add(count as u64)
                .ok_or_else(|| "更新包大小溢出。".to_string())?;
            if downloaded > checked.asset.size || downloaded > MAX_ARTIFACT_BYTES {
                return Err("更新包超过清单声明或安全大小上限。".into());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| format!("写入更新包失败：{error}"))?;
            hasher.update(&buffer[..count]);
            let mut runtime = self
                .runtime
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if runtime.status.phase == UpdatePhase::Downloading {
                runtime.status.downloaded_bytes = downloaded;
            }
        }
        if downloaded != checked.asset.size {
            return Err("更新包实际大小与清单不一致。".into());
        }
        output
            .sync_all()
            .map_err(|error| format!("无法同步更新包：{error}"))?;
        drop(output);
        let digest: [u8; 32] = hasher.finalize().into();
        verify_asset_digest_and_signature(
            &checked.version,
            checked.platform,
            checked.asset.size,
            &checked.asset.sha256,
            &checked.asset.signature,
            &digest,
        )?;
        validate_existing_update_subdirectory(
            &self.cache_dir,
            &[checked.version.as_str()],
            library_roots,
        )?;
        match fs::symlink_metadata(&destination) {
            Ok(_) => {
                ensure_safe_update_file(&destination, &self.cache_dir, library_roots, "旧更新包")?;
                fs::remove_file(&destination)
                    .map_err(|error| format!("无法替换应用更新缓存中的旧包：{error}"))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("无法检查应用更新缓存中的旧包：{error}")),
        }
        fs::rename(&partial, &destination)
            .map_err(|error| format!("无法完成更新包缓存写入：{error}"))?;
        ensure_safe_update_file(&destination, &self.cache_dir, library_roots, "更新包")?;
        std::mem::forget(cleanup);
        self.cleanup_old_version_downloads(&checked.version, library_roots);
        Ok(DownloadedUpdate {
            checked,
            path: destination,
        })
    }

    fn cleanup_old_version_downloads(&self, keep_version: &str, library_roots: &[PathBuf]) {
        if validate_existing_update_cache(&self.cache_dir, library_roots).is_err() {
            return;
        }
        let Ok(entries) = fs::read_dir(&self.cache_dir) else {
            return;
        };
        for entry in entries.flatten().take(64) {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name == keep_version || canonical_version(&name).is_err() {
                continue;
            }
            if validate_existing_update_subdirectory(&self.cache_dir, &[&name], library_roots)
                .is_err()
            {
                continue;
            }
            remove_owned_version_cache_files(&path, &name);
        }
    }

    pub(crate) fn downloaded(&self, version: &str) -> Result<DownloadedUpdate, String> {
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime
            .downloaded
            .as_ref()
            .filter(|downloaded| downloaded.checked.version == version)
            .cloned()
            .ok_or_else(|| "指定版本尚未完成并通过验证。".to_string())
    }
}

fn is_plain_directory(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    metadata_is_plain_directory(&metadata)
}

fn metadata_is_plain_directory(metadata: &fs::Metadata) -> bool {
    let is_reparse_point = {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        }
        #[cfg(not(windows))]
        {
            false
        }
    };
    plain_directory_attributes(
        metadata.file_type().is_dir(),
        metadata.file_type().is_symlink(),
        is_reparse_point,
    )
}

fn metadata_is_plain_file(metadata: &fs::Metadata) -> bool {
    let is_reparse_point = {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        }
        #[cfg(not(windows))]
        {
            false
        }
    };
    metadata.file_type().is_file() && !metadata.file_type().is_symlink() && !is_reparse_point
}

fn plain_directory_attributes(is_directory: bool, is_symlink: bool, is_reparse: bool) -> bool {
    is_directory && !is_symlink && !is_reparse
}

fn ensure_plain_update_directory(path: &Path, create: bool) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata_is_plain_directory(&metadata) {
                return Err("应用更新目录不能是文件、junction、符号链接或其他重解析点。".into());
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            fs::create_dir(path).map_err(|create_error| {
                format!(
                    "无法安全创建应用更新目录 {}：{create_error}",
                    path.display()
                )
            })?;
            ensure_plain_update_directory(path, false)
        }
        Err(error) => Err(format!("无法检查应用更新目录 {}：{error}", path.display())),
    }
}

pub(crate) fn ensure_safe_update_cache(
    cache_dir: &Path,
    library_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    validate_update_cache(cache_dir, library_roots, true)
}

pub(crate) fn validate_existing_update_cache(
    cache_dir: &Path,
    library_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    validate_update_cache(cache_dir, library_roots, false)
}

fn validate_update_cache(
    cache_dir: &Path,
    library_roots: &[PathBuf],
    create: bool,
) -> Result<PathBuf, String> {
    if !cache_dir.is_absolute() || cache_dir.parent().is_none() {
        return Err("应用更新缓存必须是非卷根的绝对路径。".into());
    }
    let resolved_before_create = cache::resolve_for_comparison(cache_dir)?;
    for root in library_roots {
        if cache::paths_overlap_checked(&resolved_before_create, root)? {
            return Err(
                "应用更新缓存与媒体资源库真实路径重叠；为保持媒体源只读，已停止更新。".into(),
            );
        }
    }
    ensure_plain_update_directory(cache_dir, create)?;
    let resolved = cache::resolve_for_comparison(cache_dir)?;
    for root in library_roots {
        if cache::paths_overlap_checked(&resolved, root)? {
            return Err(
                "应用更新缓存与媒体资源库真实路径重叠；为保持媒体源只读，已停止更新。".into(),
            );
        }
    }
    // Recheck after canonicalization so a pre-existing reparse point cannot be accepted merely
    // because its target happened to be outside the current Library Roots.
    ensure_plain_update_directory(cache_dir, false)?;
    Ok(resolved)
}

pub(crate) fn ensure_safe_update_subdirectory(
    cache_dir: &Path,
    components: &[&str],
    library_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    validate_update_subdirectory(cache_dir, components, library_roots, true)
}

pub(crate) fn validate_existing_update_subdirectory(
    cache_dir: &Path,
    components: &[&str],
    library_roots: &[PathBuf],
) -> Result<PathBuf, String> {
    validate_update_subdirectory(cache_dir, components, library_roots, false)
}

fn validate_update_subdirectory(
    cache_dir: &Path,
    components: &[&str],
    library_roots: &[PathBuf],
    create: bool,
) -> Result<PathBuf, String> {
    let resolved_cache = validate_update_cache(cache_dir, library_roots, create)?;
    let mut directory = cache_dir.to_path_buf();
    for component in components {
        let component_path = Path::new(component);
        if component.is_empty()
            || component_path.components().count() != 1
            || !matches!(
                component_path.components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err("应用更新子目录名称无效。".into());
        }
        directory.push(component);
        let resolved_before_create = cache::resolve_for_comparison(&directory)?;
        validate_resolved_update_subdirectory(
            &resolved_before_create,
            &resolved_cache,
            library_roots,
        )?;
        ensure_plain_update_directory(&directory, create)?;
        let resolved = cache::resolve_for_comparison(&directory)?;
        validate_resolved_update_subdirectory(&resolved, &resolved_cache, library_roots)?;
        ensure_plain_update_directory(&directory, false)?;
    }
    Ok(directory)
}

fn validate_resolved_update_subdirectory(
    resolved: &Path,
    resolved_cache: &Path,
    library_roots: &[PathBuf],
) -> Result<(), String> {
    if !cache::is_equal_or_within_checked(resolved, resolved_cache)? {
        return Err("应用更新子目录逃逸出应用更新缓存。".into());
    }
    for root in library_roots {
        if cache::paths_overlap_checked(resolved, root)? {
            return Err("应用更新子目录与媒体资源库真实路径重叠；已停止更新。".into());
        }
    }
    Ok(())
}

pub(crate) fn ensure_safe_update_file(
    path: &Path,
    cache_dir: &Path,
    library_roots: &[PathBuf],
    label: &str,
) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("无法检查{label}：{error}"))?;
    if !metadata_is_plain_file(&metadata) {
        return Err(format!("{label}不是安全的普通文件。"));
    }
    let resolved_cache = validate_existing_update_cache(cache_dir, library_roots)?;
    let resolved = cache::resolve_for_comparison(path)?;
    if !cache::is_equal_or_within_checked(&resolved, &resolved_cache)? {
        return Err(format!("{label}不属于应用更新缓存。"));
    }
    for root in library_roots {
        if cache::paths_overlap_checked(&resolved, root)? {
            return Err(format!("{label}与媒体资源库真实路径重叠。"));
        }
    }
    Ok(())
}

fn remove_owned_version_cache_files(version_dir: &Path, version: &str) {
    if !is_plain_directory(version_dir) {
        return;
    }
    let Ok(entries) = fs::read_dir(version_dir) else {
        return;
    };
    let expected_portable = format!("M2Shelf-Portable-{version}-x64.zip");
    let expected_nsis = format!("M2Shelf-Setup-{version}-x64.exe");
    let mut safe_to_remove_directory = true;
    for entry in entries.take(65) {
        let Ok(entry) = entry else {
            safe_to_remove_directory = false;
            break;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let owned = name == expected_portable
            || name == expected_nsis
            || is_owned_partial_download(&name, &expected_portable)
            || is_owned_partial_download(&name, &expected_nsis);
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(_) => {
                safe_to_remove_directory = false;
                continue;
            }
        };
        if !owned || !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            safe_to_remove_directory = false;
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                safe_to_remove_directory = false;
                continue;
            }
        }
        if fs::remove_file(entry.path()).is_err() {
            safe_to_remove_directory = false;
        }
    }
    if safe_to_remove_directory {
        let _ = fs::remove_dir(version_dir);
    }
}

fn cleanup_owned_partial_downloads(version_dir: &Path, asset_name: &str) {
    if !is_plain_directory(version_dir) {
        return;
    }
    let Ok(entries) = fs::read_dir(version_dir) else {
        return;
    };
    for entry in entries.take(128).flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_owned_partial_download(&name, asset_name) {
            continue;
        }
        if fs::symlink_metadata(entry.path())
            .is_ok_and(|metadata| metadata_is_plain_file(&metadata))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn is_owned_partial_download(name: &str, asset_name: &str) -> bool {
    let Some(value) = name
        .strip_prefix(&format!(".{asset_name}."))
        .and_then(|value| value.strip_suffix(".partial"))
    else {
        return false;
    };
    Uuid::parse_str(value).is_ok_and(|uuid| uuid.to_string() == value)
}

impl CheckedUpdate {
    fn to_public(&self) -> AvailableUpdate {
        AvailableUpdate {
            version: self.version.clone(),
            published_at: self.published_at.clone(),
            release_notes: self.notes.clone(),
            file_name: self.asset.file_name.clone(),
            download_size: self.asset.size,
            sha256: self.asset.sha256.clone(),
        }
    }
}

struct PartialFileGuard(PathBuf);

impl Drop for PartialFileGuard {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.0).is_ok_and(|metadata| metadata_is_plain_file(&metadata)) {
            let _ = fs::remove_file(&self.0);
        }
    }
}

fn validate_manifest(
    bytes: &[u8],
    current_version: &str,
    distribution: UpdateDistribution,
) -> Result<Option<CheckedUpdate>, String> {
    let key = configured_verifying_key()?;
    validate_manifest_with_key(bytes, current_version, distribution, &key)
}

fn validate_manifest_with_key(
    bytes: &[u8],
    current_version: &str,
    distribution: UpdateDistribution,
    key: &VerifyingKey,
) -> Result<Option<CheckedUpdate>, String> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("更新清单超过安全大小上限。".into());
    }
    let manifest: UpdateManifest =
        serde_json::from_slice(bytes).map_err(|error| format!("更新清单格式无效：{error}"))?;
    if manifest.schema_version != 1 {
        return Err("不支持此更新清单版本。".into());
    }
    let parsed_version = canonical_version(&manifest.version)?;
    let current = canonical_version(current_version)?;
    DateTime::parse_from_rfc3339(&manifest.published_at)
        .map_err(|_| "更新清单 publishedAt 不是有效的 RFC 3339 时间。".to_string())?;
    manifest.notes.validate()?;

    validate_asset_with_key(
        &manifest.platforms.portable,
        &manifest.version,
        PORTABLE_PLATFORM,
        key,
    )?;
    validate_asset_with_key(
        &manifest.platforms.nsis,
        &manifest.version,
        NSIS_PLATFORM,
        key,
    )?;
    let (platform, asset) = match distribution {
        UpdateDistribution::Portable => (PORTABLE_PLATFORM, manifest.platforms.portable),
        UpdateDistribution::Nsis => (NSIS_PLATFORM, manifest.platforms.nsis),
    };

    if parsed_version <= current {
        return Ok(None);
    }
    Ok(Some(CheckedUpdate {
        distribution,
        platform,
        version: manifest.version,
        published_at: manifest.published_at,
        notes: manifest.notes.into_map(),
        asset,
    }))
}

fn validate_asset_with_key(
    asset: &UpdateAsset,
    version: &str,
    platform: &str,
    key: &VerifyingKey,
) -> Result<(), String> {
    if asset.size == 0 || asset.size > MAX_ARTIFACT_BYTES {
        return Err("更新包大小为零或超过安全上限。".into());
    }
    if asset.file_name.is_empty()
        || asset.file_name.len() > 160
        || !asset.file_name.is_ascii()
        || asset
            .file_name
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || b"._-".contains(&byte)))
        || asset.file_name == "."
        || asset.file_name == ".."
    {
        return Err("更新包文件名不安全。".into());
    }
    let expected_file_name = if platform == PORTABLE_PLATFORM {
        format!("M2Shelf-Portable-{version}-x64.zip")
    } else if platform == NSIS_PLATFORM {
        format!("M2Shelf-Setup-{version}-x64.exe")
    } else {
        return Err("更新包平台不受支持。".into());
    };
    if asset.file_name != expected_file_name {
        return Err("更新包文件名与版本或发布平台不匹配。".into());
    }
    let expected_url = format!(
        "https://github.com/Undermori/M2Shelf/releases/download/v{version}/{}",
        asset.file_name
    );
    if asset.url != expected_url {
        return Err("更新包 URL 必须是此版本的固定 GitHub Release 地址。".into());
    }
    let url = Url::parse(&asset.url).map_err(|_| "更新包 URL 无效。".to_string())?;
    validate_initial_url(&url)?;
    let digest = decode_sha256(&asset.sha256)?;
    let signature = decode_signature(&asset.signature)?;
    verify_signature(key, version, platform, asset.size, &digest, &signature)?;
    Ok(())
}

fn validate_initial_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("更新地址不是允许的 HTTPS GitHub Release 地址。".into());
    }
    Ok(())
}

fn canonical_version(value: &str) -> Result<Version, String> {
    if value.is_empty() || value.len() > MAX_VERSION_CHARS {
        return Err("更新版本号为空或过长。".into());
    }
    let version = Version::parse(value).map_err(|_| "更新版本号不是 SemVer。".to_string())?;
    if version.to_string() != value {
        return Err("更新版本号必须使用规范 SemVer 格式。".into());
    }
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err("稳定更新通道不接受预发布或构建元数据版本。".into());
    }
    Ok(version)
}

pub fn ensure_newer_version(current: &str, candidate: &str) -> Result<(), String> {
    let current = canonical_version(current)?;
    let candidate = canonical_version(candidate)?;
    if candidate <= current {
        return Err("拒绝安装相同或更低版本。".into());
    }
    Ok(())
}

pub fn signature_message(
    version: &str,
    platform: &str,
    size: u64,
    sha256: &[u8; 32],
) -> Result<Vec<u8>, String> {
    canonical_version(version)?;
    if !matches!(platform, PORTABLE_PLATFORM | NSIS_PLATFORM) {
        return Err("签名平台无效。".into());
    }
    if size == 0 || size > MAX_ARTIFACT_BYTES {
        return Err("签名文件大小为零或超过安全上限。".into());
    }
    let mut message = Vec::with_capacity(
        SIGNATURE_DOMAIN.len() + APP_ID.len() + version.len() + platform.len() + 43,
    );
    message.extend_from_slice(SIGNATURE_DOMAIN);
    message.extend_from_slice(APP_ID.as_bytes());
    message.push(0);
    message.extend_from_slice(version.as_bytes());
    message.push(0);
    message.extend_from_slice(platform.as_bytes());
    message.push(0);
    message.extend_from_slice(&size.to_le_bytes());
    message.extend_from_slice(sha256);
    Ok(message)
}

pub fn sign_digest(
    signing_key: &SigningKey,
    version: &str,
    platform: &str,
    size: u64,
    sha256: &[u8; 32],
) -> Result<Signature, String> {
    Ok(signing_key.sign(&signature_message(version, platform, size, sha256)?))
}

pub fn verify_signature(
    verifying_key: &VerifyingKey,
    version: &str,
    platform: &str,
    size: u64,
    sha256: &[u8; 32],
    signature: &Signature,
) -> Result<(), String> {
    verifying_key
        .verify(
            &signature_message(version, platform, size, sha256)?,
            signature,
        )
        .map_err(|_| "更新包 Ed25519 签名验证失败。".to_string())
}

pub fn hash_file(path: &Path) -> Result<(u64, [u8; 32]), String> {
    let mut file = File::open(path).map_err(|error| format!("无法打开待签名文件：{error}"))?;
    hash_open_file(&mut file)
}

fn hash_open_file(file: &mut File) -> Result<(u64, [u8; 32]), String> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("无法重置待验证文件：{error}"))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("无法读取待签名文件：{error}"))?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| "待签名文件大小溢出。".to_string())?;
        if size > MAX_ARTIFACT_BYTES {
            return Err("待签名文件超过更新包大小上限。".into());
        }
        hasher.update(&buffer[..count]);
    }
    if size == 0 {
        return Err("拒绝签名空文件。".into());
    }
    let digest = hasher.finalize().into();
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("无法重置已验证文件：{error}"))?;
    Ok((size, digest))
}

pub fn decode_signing_key(value: &str) -> Result<SigningKey, String> {
    let bytes = decode_canonical_base64(value.trim(), "更新私钥")?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "更新私钥必须恰好包含 32 字节 Ed25519 seed。".to_string())?;
    Ok(SigningKey::from_bytes(&seed))
}

fn verifying_key_from_text(text: &str) -> Result<VerifyingKey, String> {
    let text = text.trim();
    if text == "UNCONFIGURED" || text.is_empty() {
        return Err("此构建尚未配置公开更新签名密钥；已安全停止更新。".into());
    }
    let bytes = decode_canonical_base64(text, "更新公钥")?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "更新公钥必须恰好包含 32 字节。".to_string())?;
    VerifyingKey::from_bytes(&key).map_err(|_| "更新公钥无效。".to_string())
}

fn ensure_signing_key_matches_text(
    signing_key: &SigningKey,
    expected_public_key_text: &str,
) -> Result<(), String> {
    let expected = verifying_key_from_text(expected_public_key_text)?;
    if signing_key.verifying_key() != expected {
        return Err("M2SHELF_UPDATE_PRIVATE_KEY 与此 helper 编译嵌入的更新公钥不匹配。".into());
    }
    Ok(())
}

pub fn encode_sha256(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub fn decode_sha256(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("更新包 SHA-256 必须是 64 位小写十六进制。".into());
    }
    let mut output = [0_u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "更新包 SHA-256 无效。".to_string())?;
    }
    Ok(output)
}

pub fn decode_signature(value: &str) -> Result<Signature, String> {
    let bytes = decode_canonical_base64(value, "更新签名")?;
    let signature: [u8; 64] = bytes
        .try_into()
        .map_err(|_| "更新签名必须恰好包含 64 字节。".to_string())?;
    Ok(Signature::from_bytes(&signature))
}

fn decode_canonical_base64(value: &str, label: &str) -> Result<Vec<u8>, String> {
    let bytes = BASE64_STANDARD
        .decode(value)
        .map_err(|_| format!("{label}不是标准 Base64。"))?;
    if BASE64_STANDARD.encode(&bytes) != value {
        return Err(format!("{label}必须使用规范的标准 Base64。"));
    }
    Ok(bytes)
}

fn configured_verifying_key() -> Result<VerifyingKey, String> {
    verifying_key_from_text(PUBLIC_KEY_TEXT)
}

fn verify_asset_digest_and_signature(
    version: &str,
    platform: &str,
    size: u64,
    expected_sha256: &str,
    encoded_signature: &str,
    digest: &[u8; 32],
) -> Result<(), String> {
    let expected = decode_sha256(expected_sha256)?;
    if expected != *digest {
        return Err("更新包 SHA-256 校验失败。".into());
    }
    let signature = decode_signature(encoded_signature)?;
    let key = configured_verifying_key()?;
    verify_signature(&key, version, platform, size, digest, &signature)
}

pub(crate) fn verify_file_against_manifest(
    path: &Path,
    version: &str,
    platform: &str,
    expected_size: u64,
    expected_sha256: &str,
    encoded_signature: &str,
) -> Result<(), String> {
    let (actual_size, digest) = hash_file(path)?;
    if actual_size != expected_size {
        return Err("更新包实际大小与清单不一致。".into());
    }
    verify_asset_digest_and_signature(
        version,
        platform,
        expected_size,
        expected_sha256,
        encoded_signature,
        &digest,
    )
}

pub(crate) struct VerifiedArtifactGuard {
    _file: File,
}

pub(crate) fn lock_and_verify_file_against_manifest(
    path: &Path,
    version: &str,
    platform: &str,
    expected_size: u64,
    expected_sha256: &str,
    encoded_signature: &str,
) -> Result<VerifiedArtifactGuard, String> {
    #[cfg(windows)]
    let mut file = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
            .map_err(|error| format!("无法锁定更新包进行最终验证：{error}"))?
    };
    #[cfg(not(windows))]
    let mut file =
        File::open(path).map_err(|error| format!("无法打开更新包进行最终验证：{error}"))?;

    verify_open_file_against_manifest(
        &mut file,
        version,
        platform,
        expected_size,
        expected_sha256,
        encoded_signature,
    )?;
    Ok(VerifiedArtifactGuard { _file: file })
}

pub(crate) fn verify_open_file_against_manifest(
    file: &mut File,
    version: &str,
    platform: &str,
    expected_size: u64,
    expected_sha256: &str,
    encoded_signature: &str,
) -> Result<(), String> {
    let (actual_size, digest) = hash_open_file(file)?;
    if actual_size != expected_size {
        return Err("更新包实际大小与清单不一致。".into());
    }
    verify_asset_digest_and_signature(
        version,
        platform,
        expected_size,
        expected_sha256,
        encoded_signature,
        &digest,
    )
}

fn http_client(timeout: Duration) -> Result<Client, String> {
    Client::builder()
        .https_only(true)
        .timeout(timeout)
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.error("更新请求重定向次数过多");
            }
            if allowed_download_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("更新请求被重定向到未授权主机")
            }
        }))
        .user_agent(concat!("M2Shelf/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("无法创建受限更新客户端：{error}"))
}

fn allowed_download_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn ensure_success_response(response: &Response, label: &str) -> Result<(), String> {
    if !response.status().is_success() {
        return Err(format!("{label}请求失败：HTTP {}。", response.status()));
    }
    if !allowed_download_url(response.url()) {
        return Err(format!("{label}最终地址不属于允许的 GitHub 主机。"));
    }
    Ok(())
}

fn read_bounded_response(response: Response, limit: u64, label: &str) -> Result<Vec<u8>, String> {
    ensure_success_response(&response, label)?;
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(format!("{label}超过安全大小上限。"));
    }
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取{label}失败：{error}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{label}超过安全大小上限。"));
    }
    Ok(bytes)
}

fn read_update_manifest(
    response: Response,
    current_version: &str,
) -> Result<Option<Vec<u8>>, String> {
    if response.status().is_success() {
        return read_bounded_response(response, MAX_MANIFEST_BYTES, "更新清单").map(Some);
    }

    // The bootstrap build can be newer than the latest public Release while that older
    // Release has no updater manifest. GitHub still redirects the fixed /latest/ URL to
    // the exact release tag before returning 404, so that tag is sufficient to prove only
    // that no newer public stable version exists. It is never used to trust or install an
    // artifact; a newer release without the signed manifest remains a hard failure.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        accept_missing_manifest_for_non_newer_release(response.url(), current_version)?;
        return Ok(None);
    }

    ensure_success_response(&response, "更新清单")?;
    unreachable!("successful responses return before ensure_success_response")
}

fn accept_missing_manifest_for_non_newer_release(
    final_url: &Url,
    current_version: &str,
) -> Result<(), String> {
    let release_version = release_version_from_missing_manifest_url(final_url)?;
    let current_version = canonical_version(current_version)?;
    if release_version > current_version {
        return Err(format!(
            "GitHub 最新稳定版本 {release_version} 缺少受签名更新清单；已安全停止更新。"
        ));
    }
    Ok(())
}

fn release_version_from_missing_manifest_url(url: &Url) -> Result<Version, String> {
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("缺失更新清单的最终地址不是允许的 GitHub Release 地址。".into());
    }

    const PREFIX: &str = "/Undermori/M2Shelf/releases/download/v";
    const SUFFIX: &str = "/latest.json";
    let path = url.path();
    let version = path
        .strip_prefix(PREFIX)
        .and_then(|remainder| remainder.strip_suffix(SUFFIX))
        .filter(|value| !value.is_empty() && !value.contains('/'))
        .ok_or_else(|| "缺失更新清单的最终地址不对应 M²Shelf 的精确版本 Release。".to_string())?;
    canonical_version(version)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedArtifactOutput {
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignerIdentityOutput {
    pub schema_version: u32,
    pub app_id: &'static str,
    pub version: &'static str,
    pub public_key: String,
}

pub fn signer_identity_for_cli() -> Result<SignerIdentityOutput, String> {
    let key = configured_verifying_key()?;
    Ok(SignerIdentityOutput {
        schema_version: 1,
        app_id: APP_ID,
        version: env!("CARGO_PKG_VERSION"),
        public_key: BASE64_STANDARD.encode(key.to_bytes()),
    })
}

pub fn sign_artifact_for_cli(
    path: &Path,
    version: &str,
    platform: &str,
    private_key: &str,
) -> Result<SignedArtifactOutput, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "待签名文件必须有有效的 Unicode 文件名。".to_string())?
        .to_owned();
    let key = decode_signing_key(private_key)?;
    ensure_signing_key_matches_text(&key, PUBLIC_KEY_TEXT)?;
    let (size, digest) = hash_file(path)?;
    let signature = sign_digest(&key, version, platform, size, &digest)?;
    Ok(SignedArtifactOutput {
        file_name,
        size,
        sha256: encode_sha256(&digest),
        signature: BASE64_STANDARD.encode(signature.to_bytes()),
    })
}

fn verify_artifact_with_key(
    path: &Path,
    version: &str,
    platform: &str,
    encoded_signature: &str,
    verifying_key: &VerifyingKey,
) -> Result<SignedArtifactOutput, String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "待验证文件必须有有效的 Unicode 文件名。".to_string())?
        .to_owned();
    let (size, digest) = hash_file(path)?;
    let signature = decode_signature(encoded_signature)?;
    verify_signature(verifying_key, version, platform, size, &digest, &signature)?;
    Ok(SignedArtifactOutput {
        file_name,
        size,
        sha256: encode_sha256(&digest),
        signature: BASE64_STANDARD.encode(signature.to_bytes()),
    })
}

pub fn verify_artifact_for_cli(
    path: &Path,
    version: &str,
    platform: &str,
    encoded_signature: &str,
) -> Result<SignedArtifactOutput, String> {
    verify_artifact_with_key(
        path,
        version,
        platform,
        encoded_signature,
        &configured_verifying_key()?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use serde_json::{json, Value};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn signed_asset(
        key: &SigningKey,
        version: &str,
        platform: &str,
        file_name: &str,
        size: u64,
        digest: [u8; 32],
    ) -> Value {
        let signature = sign_digest(key, version, platform, size, &digest).unwrap();
        json!({
            "url": format!(
                "https://github.com/Undermori/M2Shelf/releases/download/v{version}/{file_name}"
            ),
            "fileName": file_name,
            "size": size,
            "sha256": encode_sha256(&digest),
            "signature": BASE64_STANDARD.encode(signature.to_bytes()),
        })
    }

    fn signed_manifest(key: &SigningKey, version: &str) -> Value {
        let mut platforms = serde_json::Map::new();
        platforms.insert(
            PORTABLE_PLATFORM.to_owned(),
            signed_asset(
                key,
                version,
                PORTABLE_PLATFORM,
                &format!("M2Shelf-Portable-{version}-x64.zip"),
                123,
                [3_u8; 32],
            ),
        );
        platforms.insert(
            NSIS_PLATFORM.to_owned(),
            signed_asset(
                key,
                version,
                NSIS_PLATFORM,
                &format!("M2Shelf-Setup-{version}-x64.exe"),
                456,
                [4_u8; 32],
            ),
        );
        json!({
            "schemaVersion": 1,
            "version": version,
            "publishedAt": "2026-08-24T12:00:00Z",
            "notes": {
                "zh-CN": "测试版本",
                "en-US": "Test release",
                "ja-JP": "テスト版",
                "ko-KR": "테스트 릴리스"
            },
            "platforms": platforms
        })
    }

    #[test]
    fn signature_binds_version_and_platform() {
        let key = signing_key();
        let digest = [3_u8; 32];
        let signature = sign_digest(&key, "1.2.3", PORTABLE_PLATFORM, 123, &digest).unwrap();
        let verifying = key.verifying_key();
        verify_signature(
            &verifying,
            "1.2.3",
            PORTABLE_PLATFORM,
            123,
            &digest,
            &signature,
        )
        .unwrap();
        assert!(verify_signature(
            &verifying,
            "1.2.4",
            PORTABLE_PLATFORM,
            123,
            &digest,
            &signature
        )
        .is_err());
        assert!(
            verify_signature(&verifying, "1.2.3", NSIS_PLATFORM, 123, &digest, &signature).is_err()
        );
        assert!(verify_signature(
            &verifying,
            "1.2.3",
            PORTABLE_PLATFORM,
            124,
            &digest,
            &signature
        )
        .is_err());
        assert!(verify_signature(
            &verifying,
            "1.2.3",
            PORTABLE_PLATFORM,
            123,
            &[4_u8; 32],
            &signature
        )
        .is_err());
    }

    #[test]
    fn strict_signed_manifest_accepts_only_a_newer_fixed_release() {
        let key = signing_key();
        let manifest = signed_manifest(&key, "1.2.4");
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let checked = validate_manifest_with_key(
            &bytes,
            "1.2.3",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(checked.version, "1.2.4");
        assert_eq!(checked.platform, PORTABLE_PLATFORM);

        assert!(validate_manifest_with_key(
            &bytes,
            "1.2.4",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .unwrap()
        .is_none());
        assert!(validate_manifest_with_key(
            &bytes,
            "1.2.5",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn manifest_rejects_unknown_fields_bad_urls_and_noncanonical_versions() {
        let key = signing_key();

        let mut unknown = signed_manifest(&key, "1.2.4");
        unknown["unexpected"] = json!(true);
        assert!(validate_manifest_with_key(
            &serde_json::to_vec(&unknown).unwrap(),
            "1.2.3",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .is_err());

        let mut bad_url = signed_manifest(&key, "1.2.4");
        bad_url["platforms"][PORTABLE_PLATFORM]["url"] = json!("https://example.com/M2Shelf.zip");
        assert!(validate_manifest_with_key(
            &serde_json::to_vec(&bad_url).unwrap(),
            "1.2.3",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .is_err());

        let mut noncanonical_file_name = signed_manifest(&key, "1.2.4");
        noncanonical_file_name["platforms"][PORTABLE_PLATFORM]["fileName"] =
            json!("M2Shelf-Portable-latest-x64.zip");
        noncanonical_file_name["platforms"][PORTABLE_PLATFORM]["url"] = json!(
            "https://github.com/Undermori/M2Shelf/releases/download/v1.2.4/M2Shelf-Portable-latest-x64.zip"
        );
        assert!(validate_manifest_with_key(
            &serde_json::to_vec(&noncanonical_file_name).unwrap(),
            "1.2.3",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .is_err());

        let mut noncanonical = signed_manifest(&key, "1.2.4");
        noncanonical["version"] = json!("01.2.4");
        assert!(validate_manifest_with_key(
            &serde_json::to_vec(&noncanonical).unwrap(),
            "1.2.3",
            UpdateDistribution::Portable,
            &key.verifying_key(),
        )
        .is_err());
    }

    #[test]
    fn canonical_semver_and_no_downgrade_are_enforced() {
        assert!(canonical_version("1.2.3").is_ok());
        assert!(canonical_version("01.2.3").is_err());
        assert!(canonical_version("v1.2.3").is_err());
        assert!(canonical_version("1.2.3-beta.1").is_err());
        assert!(canonical_version("1.2.3+build.1").is_err());
        assert!(ensure_newer_version("1.2.3", "1.2.4").is_ok());
        assert!(ensure_newer_version("1.2.3", "1.2.3").is_err());
        assert!(ensure_newer_version("1.2.3", "1.2.2").is_err());
    }

    #[test]
    fn update_cache_overlap_is_rejected_before_any_directory_is_created() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        fs::create_dir(&library).unwrap();
        let sentinel = library.join("sentinel.bin");
        fs::write(&sentinel, b"unchanged").unwrap();
        let update_cache = library.join("app-data").join("updates");

        assert!(ensure_safe_update_cache(&update_cache, std::slice::from_ref(&library)).is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
        assert!(!library.join("app-data").exists());
        assert!(!update_cache.exists());
    }

    #[test]
    fn unsafe_existing_update_subdirectory_is_rejected_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("updates");
        ensure_safe_update_cache(&cache, &[]).unwrap();
        let transactions = cache.join("transactions");
        fs::write(&transactions, b"not a directory").unwrap();

        assert!(ensure_safe_update_subdirectory(&cache, &["transactions", "child"], &[]).is_err());
        assert_eq!(fs::read(&transactions).unwrap(), b"not a directory");
        assert!(!cache.join("transactions/child").exists());
    }

    #[test]
    fn a_new_download_removes_only_its_canonical_plain_partial_files() {
        let temp = tempfile::tempdir().unwrap();
        let version_dir = temp.path().join("0.5.9");
        fs::create_dir(&version_dir).unwrap();
        let asset = "M2Shelf-Portable-0.5.9-x64.zip";
        let owned = version_dir.join(format!(".{asset}.{}.partial", Uuid::new_v4()));
        let other_asset = version_dir.join(format!(
            ".M2Shelf-Setup-0.5.9-x64.exe.{}.partial",
            Uuid::new_v4()
        ));
        let noncanonical = version_dir.join(format!(".{asset}.NOT-A-UUID.partial"));
        let unknown = version_dir.join("notes.txt");
        let directory_lookalike = version_dir.join(format!(".{asset}.{}.partial", Uuid::new_v4()));
        fs::write(&owned, b"partial").unwrap();
        fs::write(&other_asset, b"other").unwrap();
        fs::write(&noncanonical, b"invalid").unwrap();
        fs::write(&unknown, b"keep").unwrap();
        fs::create_dir(&directory_lookalike).unwrap();

        cleanup_owned_partial_downloads(&version_dir, asset);

        assert!(!owned.exists());
        assert_eq!(fs::read(other_asset).unwrap(), b"other");
        assert_eq!(fs::read(noncanonical).unwrap(), b"invalid");
        assert_eq!(fs::read(unknown).unwrap(), b"keep");
        assert!(directory_lookalike.is_dir());
    }

    #[test]
    fn plain_directory_attribute_gate_rejects_symlinks_and_windows_reparse_points() {
        assert!(plain_directory_attributes(true, false, false));
        assert!(!plain_directory_attributes(false, false, false));
        assert!(!plain_directory_attributes(true, true, false));
        assert!(!plain_directory_attributes(true, false, true));
    }

    #[test]
    fn missing_manifest_is_accepted_only_for_an_exact_non_newer_release_url() {
        let older =
            Url::parse("https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/latest.json")
                .unwrap();
        let equal =
            Url::parse("https://github.com/Undermori/M2Shelf/releases/download/v0.5.8/latest.json")
                .unwrap();
        let newer =
            Url::parse("https://github.com/Undermori/M2Shelf/releases/download/v0.5.9/latest.json")
                .unwrap();

        assert!(accept_missing_manifest_for_non_newer_release(&older, "0.5.8").is_ok());
        assert!(accept_missing_manifest_for_non_newer_release(&equal, "0.5.8").is_ok());
        assert!(accept_missing_manifest_for_non_newer_release(&newer, "0.5.8").is_err());
    }

    #[test]
    fn missing_manifest_fallback_rejects_ambiguous_or_untrusted_urls() {
        for value in [
            "http://github.com/Undermori/M2Shelf/releases/download/v0.5.7/latest.json",
            "https://example.com/Undermori/M2Shelf/releases/download/v0.5.7/latest.json",
            "https://github.com/undermori/M2Shelf/releases/download/v0.5.7/latest.json",
            "https://github.com/Undermori/Other/releases/download/v0.5.7/latest.json",
            "https://github.com/Undermori/M2Shelf/releases/latest/download/latest.json",
            "https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/latest.json?download=1",
            "https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/latest.json#fragment",
            "https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/extra/latest.json",
            "https://github.com/Undermori/M2Shelf/releases/download/v0.5.7-beta.1/latest.json",
            "https://github.com/Undermori/M2Shelf/releases/download/v01.5.7/latest.json",
            "https://github.com/Undermori/M2Shelf/releases/download/v0.5.7/LATEST.json",
            "https://github.com/Undermori/M2Shelf/releases/download/v0.5.7%2Flatest/latest.json",
        ] {
            let url = Url::parse(value).unwrap();
            assert!(
                release_version_from_missing_manifest_url(&url).is_err(),
                "unexpectedly accepted {value}"
            );
        }
    }

    #[test]
    fn sha_and_signature_encodings_are_strict() {
        assert!(decode_sha256(&"a".repeat(64)).is_ok());
        assert!(decode_sha256(&"A".repeat(64)).is_err());
        assert!(decode_sha256(&"a".repeat(63)).is_err());
        let signature = BASE64_STANDARD.encode([0_u8; 64]);
        assert!(decode_signature(&signature).is_ok());
        assert!(decode_signature(&BASE64_STANDARD.encode([0_u8; 63])).is_err());
        assert!(decode_signature(signature.trim_end_matches('=')).is_err());
    }

    #[test]
    fn signing_key_must_match_embedded_public_key_text() {
        let key = signing_key();
        let matching = BASE64_STANDARD.encode(key.verifying_key().to_bytes());
        let other = BASE64_STANDARD.encode(
            SigningKey::from_bytes(&[8_u8; 32])
                .verifying_key()
                .to_bytes(),
        );
        assert!(ensure_signing_key_matches_text(&key, &matching).is_ok());
        assert!(ensure_signing_key_matches_text(&key, &other).is_err());
        assert!(ensure_signing_key_matches_text(&key, "UNCONFIGURED").is_err());
    }

    #[test]
    fn cli_verifier_rehashes_the_file_and_rejects_mismatched_signatures() {
        let key = signing_key();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("M2Shelf-Portable-1.2.3-x64.zip");
        fs::write(&path, b"verified offline release artifact").unwrap();
        let (size, digest) = hash_file(&path).unwrap();
        let signature = sign_digest(&key, "1.2.3", PORTABLE_PLATFORM, size, &digest).unwrap();
        let encoded_signature = BASE64_STANDARD.encode(signature.to_bytes());

        let output = verify_artifact_with_key(
            &path,
            "1.2.3",
            PORTABLE_PLATFORM,
            &encoded_signature,
            &key.verifying_key(),
        )
        .unwrap();
        assert_eq!(output.file_name, "M2Shelf-Portable-1.2.3-x64.zip");
        assert_eq!(output.size, size);
        assert_eq!(output.sha256, encode_sha256(&digest));
        assert_eq!(output.signature, encoded_signature);
        let mut output_properties = serde_json::to_value(&output)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        output_properties.sort();
        assert_eq!(
            output_properties,
            vec!["fileName", "sha256", "signature", "size"]
        );

        assert!(verify_artifact_with_key(
            &path,
            "1.2.4",
            PORTABLE_PLATFORM,
            &encoded_signature,
            &key.verifying_key(),
        )
        .is_err());
        assert!(verify_artifact_with_key(
            &path,
            "1.2.3",
            NSIS_PLATFORM,
            &encoded_signature,
            &key.verifying_key(),
        )
        .is_err());

        fs::write(&path, b"modified offline release artifact").unwrap();
        assert!(verify_artifact_with_key(
            &path,
            "1.2.3",
            PORTABLE_PLATFORM,
            &encoded_signature,
            &key.verifying_key(),
        )
        .is_err());
    }
}
