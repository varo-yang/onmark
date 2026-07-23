//! Closed desktop target matrix and native executable-format admission.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::PackageError;

const ELF_HEADER_BYTES: usize = 20;
const MACH_O_HEADER_BYTES: usize = 8;
const PE_DOS_HEADER_BYTES: usize = 64;
const PE_SIGNATURE_BYTES: usize = 6;
const RELEASE_CONTRACT: &str = "packages/launcher/desktop-release.json";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum ReleaseTarget {
    DarwinArm64,
    LinuxX64,
    Win32X64,
}

#[derive(Clone, Copy)]
pub(super) enum ExecutableRole {
    Onmark,
    Ffmpeg,
    Ffprobe,
}

impl ExecutableRole {
    const fn stem(self) -> &'static str {
        match self {
            Self::Onmark => "onmark",
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }

    const fn invalid_reason(self) -> &'static str {
        match self {
            Self::Onmark => "invalid native Onmark executable",
            Self::Ffmpeg => "invalid FFmpeg executable",
            Self::Ffprobe => "invalid ffprobe executable",
        }
    }
}

impl ReleaseTarget {
    const ALL: [Self; 3] = [Self::DarwinArm64, Self::LinuxX64, Self::Win32X64];

    pub(super) fn parse(value: &str) -> Result<Self, PackageError> {
        match value {
            "darwin-arm64" => Ok(Self::DarwinArm64),
            "linux-x64" => Ok(Self::LinuxX64),
            "win32-x64" => Ok(Self::Win32X64),
            _ => Err(PackageError::InvalidOptions(
                format!("unsupported target {value}").into_boxed_str(),
            )),
        }
    }

    pub(super) const fn package_name(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "@onmark/cli-darwin-arm64",
            Self::LinuxX64 => "@onmark/cli-linux-x64",
            Self::Win32X64 => "@onmark/cli-win32-x64",
        }
    }

    pub(super) fn validate_contract(repository: &Path) -> Result<(), PackageError> {
        let path = repository.join(RELEASE_CONTRACT);
        let contents = fs::read(&path)
            .map_err(|source| PackageError::io("read desktop release contract", &path, source))?;
        let contract: DesktopReleaseContract = serde_json::from_slice(&contents)?;
        let expected = Self::ALL.map(Self::as_str);
        let actual = contract
            .targets
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if contract.schema_version != 1
            || contract.browser_build.trim().is_empty()
            || actual != expected
        {
            return Err(PackageError::invalid_input(
                &path,
                "desktop release contract does not match the native target matrix",
            ));
        }
        Ok(())
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "darwin-arm64",
            Self::LinuxX64 => "linux-x64",
            Self::Win32X64 => "win32-x64",
        }
    }

    pub(super) const fn operating_system(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "darwin",
            Self::LinuxX64 => "linux",
            Self::Win32X64 => "win32",
        }
    }

    pub(super) const fn architecture(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "arm64",
            Self::LinuxX64 | Self::Win32X64 => "x64",
        }
    }

    pub(super) const fn executable_name(self, role: ExecutableRole) -> &'static str {
        match (self, role) {
            (Self::Win32X64, ExecutableRole::Onmark) => "onmark.exe",
            (Self::Win32X64, ExecutableRole::Ffmpeg) => "ffmpeg.exe",
            (Self::Win32X64, ExecutableRole::Ffprobe) => "ffprobe.exe",
            (_, role) => role.stem(),
        }
    }

    pub(super) fn validate_executable(
        self,
        path: &Path,
        role: ExecutableRole,
    ) -> Result<u64, PackageError> {
        let metadata = executable_metadata(path, role)?;
        match self {
            Self::DarwinArm64 => require_mach_o_arm64(path, role)?,
            Self::LinuxX64 => require_elf_x64(path, role)?,
            Self::Win32X64 => require_pe_x64(path, role)?,
        }
        Ok(metadata.len())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DesktopReleaseContract {
    schema_version: u8,
    browser_build: Box<str>,
    targets: BTreeMap<String, serde_json::Value>,
}

fn executable_metadata(path: &Path, role: ExecutableRole) -> Result<fs::Metadata, PackageError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| PackageError::io("inspect release input", path, source))?;
    if !metadata.is_file() || !has_executable_bit(&metadata) {
        return Err(PackageError::invalid_input(path, role.invalid_reason()));
    }
    Ok(metadata)
}

fn require_mach_o_arm64(path: &Path, role: ExecutableRole) -> Result<(), PackageError> {
    let header = read_header::<MACH_O_HEADER_BYTES>(path)?;
    let cpu = u32::from_le_bytes(header[4..8].try_into().expect("the header has eight bytes"));
    let valid = header[..4] == [0xcf, 0xfa, 0xed, 0xfe] && cpu == 0x0100_000c;
    require_format(valid, path, role)
}

fn require_elf_x64(path: &Path, role: ExecutableRole) -> Result<(), PackageError> {
    let header = read_header::<ELF_HEADER_BYTES>(path)?;
    let machine = u16::from_le_bytes([header[18], header[19]]);
    let valid = header[..4] == [0x7f, b'E', b'L', b'F']
        && header[4] == 2
        && header[5] == 1
        && machine == 62;
    require_format(valid, path, role)
}

fn require_pe_x64(path: &Path, role: ExecutableRole) -> Result<(), PackageError> {
    let dos = read_header::<PE_DOS_HEADER_BYTES>(path)?;
    if dos[..2] != *b"MZ" {
        return require_format(false, path, role);
    }

    let offset = u32::from_le_bytes(dos[60..64].try_into().expect("the DOS header has 64 bytes"));
    let mut file = File::open(path)
        .map_err(|source| PackageError::io("open release executable", path, source))?;
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(|source| PackageError::io("seek release executable header", path, source))?;
    let mut signature = [0_u8; PE_SIGNATURE_BYTES];
    file.read_exact(&mut signature)
        .map_err(|source| PackageError::io("read release executable header", path, source))?;

    let machine = u16::from_le_bytes([signature[4], signature[5]]);
    require_format(
        signature[..4] == *b"PE\0\0" && machine == 0x8664,
        path,
        role,
    )
}

fn read_header<const N: usize>(path: &Path) -> Result<[u8; N], PackageError> {
    let mut file = File::open(path)
        .map_err(|source| PackageError::io("open release executable", path, source))?;
    let mut header = [0_u8; N];
    file.read_exact(&mut header)
        .map_err(|source| PackageError::io("read release executable header", path, source))?;
    Ok(header)
}

fn require_format(valid: bool, path: &Path, role: ExecutableRole) -> Result<(), PackageError> {
    if valid {
        return Ok(());
    }
    Err(PackageError::invalid_input(path, role.invalid_reason()))
}

#[cfg(unix)]
fn has_executable_bit(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn has_executable_bit(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{ExecutableRole, ReleaseTarget};

    #[test]
    fn admits_only_the_declared_binary_format() {
        let root = TempDir::new().expect("the binary fixture root is created");
        let mach_o = root.path().join("mach-o");
        let elf = root.path().join("elf");
        let pe = root.path().join("pe.exe");
        write_executable(&mach_o, &mach_o_arm64());
        write_executable(&elf, &elf_x64());
        write_executable(&pe, &pe_x64());

        ReleaseTarget::DarwinArm64
            .validate_executable(&mach_o, ExecutableRole::Onmark)
            .expect("the Mach-O target admits arm64 Mach-O");
        ReleaseTarget::LinuxX64
            .validate_executable(&elf, ExecutableRole::Ffmpeg)
            .expect("the Linux target admits x64 ELF");
        ReleaseTarget::Win32X64
            .validate_executable(&pe, ExecutableRole::Ffprobe)
            .expect("the Windows target admits x64 PE");

        assert!(
            ReleaseTarget::LinuxX64
                .validate_executable(&mach_o, ExecutableRole::Onmark)
                .is_err()
        );
        assert!(
            ReleaseTarget::Win32X64
                .validate_executable(&elf, ExecutableRole::Onmark)
                .is_err()
        );
    }

    #[test]
    fn requires_the_browser_contract_to_cover_the_native_matrix() {
        let root = TempDir::new().expect("the release contract root is created");
        let directory = root.path().join("packages/launcher");
        fs::create_dir_all(&directory).expect("the launcher directory is created");
        fs::write(
            directory.join("desktop-release.json"),
            r#"{
              "schemaVersion": 1,
              "browserBuild": "149.0.7827.55",
              "targets": {
                "darwin-arm64": {},
                "linux-x64": {},
                "win32-x64": {}
              }
            }"#,
        )
        .expect("the release contract is written");

        ReleaseTarget::validate_contract(root.path())
            .expect("the browser and native target matrices agree");

        fs::write(
            directory.join("desktop-release.json"),
            r#"{
              "schemaVersion": 1,
              "browserBuild": "149.0.7827.55",
              "targets": {"darwin-arm64": {}}
            }"#,
        )
        .expect("the incomplete release contract is written");
        assert!(ReleaseTarget::validate_contract(root.path()).is_err());
    }

    #[test]
    fn repository_browser_contract_matches_the_native_matrix() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the xtask package is below the repository root");

        ReleaseTarget::validate_contract(repository)
            .expect("the checked-in browser and native target matrices agree");
    }

    fn mach_o_arm64() -> [u8; 8] {
        [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]
    }

    fn elf_x64() -> [u8; 20] {
        let mut header = [0_u8; 20];
        header[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
        header[18..20].copy_from_slice(&62_u16.to_le_bytes());
        header
    }

    fn pe_x64() -> [u8; 70] {
        let mut header = [0_u8; 70];
        header[..2].copy_from_slice(b"MZ");
        header[60..64].copy_from_slice(&64_u32.to_le_bytes());
        header[64..68].copy_from_slice(b"PE\0\0");
        header[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
        header
    }

    fn write_executable(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).expect("the binary fixture is written");
        set_fixture_executable(path);
    }

    #[cfg(unix)]
    fn set_fixture_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt as _;

        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("the binary fixture is executable");
    }

    #[cfg(not(unix))]
    fn set_fixture_executable(_path: &Path) {}
}
