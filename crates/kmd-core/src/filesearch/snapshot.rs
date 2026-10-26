//! 파일 색인 스냅숏 — 열 배열을 그대로 디스크에, 읽을 때는 mmap (docs/22 §3.4·§3.5)
//!
//! 런처는 단축키마다 새 프로세스다. 예전에는 색인을 역직렬화해 검색 엔진을 다시
//! 만드느라 40만 개에 0.57초가 들었다. 스냅숏은 메모리 배치 그대로라 매핑만 하면
//! 바로 [`Table`] 이 된다.
//!
//! **한 번 쓴 스냅숏은 고치지 않는다.** 세대마다 새 파일(`files.<gen>.snap`)을 쓰고
//! `files.current` 가 현재 세대를 가리킨다. 매핑 중인 파일을 덮어쓰면 Unix 는 SIGBUS,
//! Windows 는 rename 자체가 막힌다 — 고친 파일을 매핑하는 일이 없게 하면 둘 다 피한다.
//!
//! **스냅숏은 신뢰하지 않는 입력으로 다룬다.** 열 때 헤더 체크섬·열 길이·오프셋 단조성·
//! 부모 순서를 검사하고, 하나라도 어긋나면 열지 않는다(호출자는 옛 경로로).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::table::{RootInfo, Table, TableBuilder, NO_PARENT};

const MAGIC: &[u8; 8] = b"KMDFS\0v1";
const VERSION: u32 = 1;
const ENDIAN_MARK: u32 = 0x0102_0304;
const HEADER_LEN: usize = 64;
const NCOLS: usize = 10;
const DIR_ENTRY_LEN: usize = 24;
const DATA_START: usize = HEADER_LEN + NCOLS * DIR_ENTRY_LEN;
const CURRENT_FILE: &str = "files.current";
/// 현재 세대에서 빠진 지 이만큼 지난 옛 세대만 지운다 — 막 연 런처가 아직 매핑 중일 수 있다
const KEEP_OLD_SECS: u64 = 10 * 60;

#[cfg(windows)]
const PLATFORM: u32 = 1;
#[cfg(not(windows))]
const PLATFORM: u32 = 0;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("스냅숏 입출력: {0}")]
    Io(#[from] std::io::Error),
    #[error("스냅숏 형식 오류: {0}")]
    Invalid(&'static str),
    #[error("빅 엔디언 기계는 스냅숏을 쓰지 않는다")]
    BigEndian,
}

/// 열 순서 — 형식의 일부, 바꾸면 VERSION 을 올린다
#[derive(Clone, Copy)]
enum Col {
    Raw,
    RawOff,
    Folded,
    FoldedOff,
    Parent,
    RootId,
    Bag,
    Meta,
    Mtime,
    Roots,
}

/// 스냅숏 기본 위치 — 로밍되지 않는 사용자 데이터 폴더 (docs/22 SEC-03).
/// 포터블 모드여도 USB 등 포터블 폴더에 파일 이름 목록을 두지 않는다.
pub fn default_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|d| d.join("kmd"))
        .unwrap_or_else(crate::Config::standard_data_dir)
        .join("filesearch")
}

/// 새 세대를 쓰고 `files.current` 를 그것으로 바꾼다. 돌려주는 값은 세대 번호.
pub fn publish(dir: &Path, table: &TableBuilder) -> Result<u64, SnapshotError> {
    if cfg!(target_endian = "big") {
        return Err(SnapshotError::BigEndian);
    }
    create_private_dir(dir)?;
    let generation = current_generation(dir).map_or(1, |g| g + 1);
    let name = format!("files.{generation}.snap");
    write_new_file(dir, &name, &encode(table, generation)?)?;
    write_new_file(dir, CURRENT_FILE, name.as_bytes())?;
    cleanup_old(dir, &name);
    Ok(generation)
}

/// `files.current` 가 가리키는 세대 번호
pub fn current_generation(dir: &Path) -> Option<u64> {
    let name = fs::read_to_string(dir.join(CURRENT_FILE)).ok()?;
    name.trim()
        .strip_prefix("files.")?
        .strip_suffix(".snap")?
        .parse()
        .ok()
}

/// 매핑된 스냅숏
pub struct Snapshot {
    map: memmap2::Mmap,
    cols: [(usize, usize); NCOLS],
    roots: Vec<RootInfo>,
    generation: u64,
    path: PathBuf,
}

impl Snapshot {
    /// `files.current` 가 가리키는 세대를 연다
    pub fn open_current(dir: &Path) -> Result<Self, SnapshotError> {
        let name = fs::read_to_string(dir.join(CURRENT_FILE))?;
        let name = name.trim();
        if name.contains(['/', '\\']) || !name.starts_with("files.") {
            return Err(SnapshotError::Invalid("current 파일 이름"));
        }
        Self::open(&dir.join(name))
    }

    pub fn open(path: &Path) -> Result<Self, SnapshotError> {
        if cfg!(target_endian = "big") {
            return Err(SnapshotError::BigEndian);
        }
        let file = File::open(path)?;
        // SAFETY: 스냅숏은 한 번 쓰면 고치지 않는다(세대별 새 파일). 열기 전후로 다른
        // 프로세스가 같은 파일을 고치는 경로가 없으므로 매핑된 바이트가 바뀌지 않는다.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        let (_rows, cols, generation) = parse_layout(&map)?;
        let roots_bytes = &map[cols[Col::Roots as usize].0..][..cols[Col::Roots as usize].1];
        let roots: Vec<RootInfo> = serde_json::from_slice(roots_bytes)
            .map_err(|_| SnapshotError::Invalid("검색 폴더 표"))?;
        let snap = Self {
            map,
            cols,
            roots,
            generation,
            path: path.to_path_buf(),
        };
        validate(&snap.table())?;
        Ok(snap)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn table(&self) -> Table<'_> {
        Table {
            raw: self.bytes(Col::Raw),
            raw_off: self.slice(Col::RawOff),
            folded: self.bytes(Col::Folded),
            folded_off: self.slice(Col::FoldedOff),
            parent: self.slice(Col::Parent),
            root_id: self.slice(Col::RootId),
            bag: self.slice(Col::Bag),
            meta: self.slice(Col::Meta),
            mtime: self.slice(Col::Mtime),
            roots: &self.roots,
        }
    }

    fn bytes(&self, col: Col) -> &[u8] {
        let (off, len) = self.cols[col as usize];
        &self.map[off..off + len]
    }

    fn slice<T: Pod>(&self, col: Col) -> &[T] {
        let bytes = self.bytes(col);
        // SAFETY: parse_layout 가 열 시작을 8바이트 정렬·길이를 원소 크기의 배수로 확인했고,
        // mmap 시작은 페이지 정렬이다. T 는 모든 비트 패턴이 유효한 정수형(Pod)뿐이다.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<T>(), bytes.len() / T::SIZE) }
    }

    /// 열 데이터 전체 체크섬까지 확인 — 쓰기 직후 검증·테스트용 (런처는 매번 하지 않는다)
    pub fn verify_data(&self) -> Result<(), SnapshotError> {
        for (i, &(off, len)) in self.cols.iter().enumerate() {
            let entry = HEADER_LEN + i * DIR_ENTRY_LEN;
            let want = read_u64(&self.map, entry + 16);
            if fnv1a(&self.map[off..off + len]) != want {
                return Err(SnapshotError::Invalid("열 체크섬"));
            }
        }
        Ok(())
    }
}

/// 열 원소로 쓸 수 있는 정수형 — 모든 비트 패턴이 유효하다
trait Pod: Copy {
    const SIZE: usize;
}
impl Pod for u16 {
    const SIZE: usize = 2;
}
impl Pod for u32 {
    const SIZE: usize = 4;
}
impl Pod for u64 {
    const SIZE: usize = 8;
}
impl Pod for i64 {
    const SIZE: usize = 8;
}

fn encode(t: &TableBuilder, generation: u64) -> Result<Vec<u8>, SnapshotError> {
    let roots = serde_json::to_vec(&t.roots).map_err(|_| SnapshotError::Invalid("검색 폴더 표"))?;
    let cols: [Vec<u8>; NCOLS] = [
        t.raw.clone(),
        le_bytes(&t.raw_off, |v| v.to_le_bytes()),
        t.folded.clone(),
        le_bytes(&t.folded_off, |v| v.to_le_bytes()),
        le_bytes(&t.parent, |v| v.to_le_bytes()),
        le_bytes(&t.root_id, |v| v.to_le_bytes()),
        le_bytes(&t.bag, |v| v.to_le_bytes()),
        le_bytes(&t.meta, |v| v.to_le_bytes()),
        le_bytes(&t.mtime, |v| v.to_le_bytes()),
        roots,
    ];

    let mut out = vec![0u8; DATA_START];
    let mut dir = Vec::with_capacity(NCOLS * DIR_ENTRY_LEN);
    for col in &cols {
        while !out.len().is_multiple_of(8) {
            out.push(0);
        }
        dir.extend_from_slice(&(out.len() as u64).to_le_bytes());
        dir.extend_from_slice(&(col.len() as u64).to_le_bytes());
        dir.extend_from_slice(&fnv1a(col).to_le_bytes());
        out.extend_from_slice(col);
    }

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&VERSION.to_le_bytes());
    header.extend_from_slice(&PLATFORM.to_le_bytes());
    header.extend_from_slice(&ENDIAN_MARK.to_le_bytes());
    header.extend_from_slice(&(t.len() as u32).to_le_bytes());
    header.extend_from_slice(&(NCOLS as u32).to_le_bytes());
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&generation.to_le_bytes());
    header.extend_from_slice(&0u64.to_le_bytes()); // 예약
    debug_assert_eq!(header.len(), 48);
    let check = fnv1a(&[header.as_slice(), &dir].concat());
    header.extend_from_slice(&check.to_le_bytes());
    header.resize(HEADER_LEN, 0);

    out[..HEADER_LEN].copy_from_slice(&header);
    out[HEADER_LEN..DATA_START].copy_from_slice(&dir);
    Ok(out)
}

fn le_bytes<T: Copy, const N: usize>(v: &[T], f: impl Fn(T) -> [u8; N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * N);
    for &x in v {
        out.extend_from_slice(&f(x));
    }
    out
}

type Layout = (usize, [(usize, usize); NCOLS], u64);

fn parse_layout(b: &[u8]) -> Result<Layout, SnapshotError> {
    use SnapshotError::Invalid;
    if b.len() < DATA_START || &b[..8] != MAGIC {
        return Err(Invalid("머리"));
    }
    if read_u32(b, 8) != VERSION {
        return Err(Invalid("형식 버전"));
    }
    if read_u32(b, 12) != PLATFORM {
        return Err(Invalid("다른 OS 의 스냅숏"));
    }
    if read_u32(b, 16) != ENDIAN_MARK {
        return Err(Invalid("엔디언"));
    }
    if read_u32(b, 24) as usize != NCOLS {
        return Err(Invalid("열 개수"));
    }
    let check = fnv1a(&[&b[..48], &b[HEADER_LEN..DATA_START]].concat());
    if read_u64(b, 48) != check {
        return Err(Invalid("머리 체크섬"));
    }
    let rows = read_u32(b, 20) as usize;
    let generation = read_u64(b, 32);

    let elem = [1usize, 4, 1, 4, 4, 2, 8, 4, 8, 1];
    let want_len = [
        None,
        Some(rows + 1),
        None,
        Some(rows + 1),
        Some(rows),
        Some(rows),
        Some(rows),
        Some(rows),
        Some(rows),
        None,
    ];
    let mut cols = [(0usize, 0usize); NCOLS];
    for i in 0..NCOLS {
        let e = HEADER_LEN + i * DIR_ENTRY_LEN;
        let off = usize::try_from(read_u64(b, e)).map_err(|_| Invalid("열 위치"))?;
        let len = usize::try_from(read_u64(b, e + 8)).map_err(|_| Invalid("열 길이"))?;
        if off % 8 != 0 || off < DATA_START || off.checked_add(len).is_none_or(|end| end > b.len())
        {
            return Err(Invalid("열 범위"));
        }
        if len % elem[i] != 0 || want_len[i].is_some_and(|n| len / elem[i] != n) {
            return Err(Invalid("열 길이"));
        }
        cols[i] = (off, len);
    }
    Ok((rows, cols, generation))
}

/// 질의가 기대는 불변식 — 범위 밖 읽기·무한 루프가 나지 않게 한다
fn validate(t: &Table<'_>) -> Result<(), SnapshotError> {
    use SnapshotError::Invalid;
    let n = t.len();
    let offsets_ok = |off: &[u32], blob: usize, min_step: u32| {
        off.first() == Some(&0)
            && off.last().is_some_and(|&l| l as usize == blob)
            && off
                .windows(2)
                .all(|w| w[1] >= w[0].saturating_add(min_step))
    };
    if !offsets_ok(t.raw_off, t.raw.len(), 0) {
        return Err(Invalid("원본 이름 오프셋"));
    }
    // 접은 이름은 구분 바이트(0)로 끝나야 한다 — folded_name 이 off-1 을 쓴다
    if !offsets_ok(t.folded_off, t.folded.len(), 1)
        || t.folded_off[1..]
            .iter()
            .any(|&o| t.folded[o as usize - 1] != 0)
    {
        return Err(Invalid("검색 이름 오프셋"));
    }
    // 부모가 자식보다 먼저 — 경로 복원이 끝나고, 조상 전파가 한 번에 된다
    if t.parent
        .iter()
        .enumerate()
        .any(|(row, &p)| p != NO_PARENT && p as usize >= row)
    {
        return Err(Invalid("부모 순서"));
    }
    if t.root_id.iter().any(|&r| r as usize >= t.roots.len()) {
        return Err(Invalid("검색 폴더 번호"));
    }
    debug_assert_eq!(t.meta.len(), n);
    Ok(())
}

fn read_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4바이트"))
}

fn read_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8바이트"))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// 사용자 전용 디렉터리 — Unix 0700, 소유자가 다르면 쓰지 않는다 (SEC-03)
fn create_private_dir(dir: &Path) -> Result<(), SnapshotError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        if !dir.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        // SAFETY: geteuid 는 실패하지 않는 순수 조회다
        let me = unsafe { libc::geteuid() };
        if fs::metadata(dir)?.uid() != me {
            return Err(SnapshotError::Invalid(
                "색인 폴더 소유자가 현재 사용자가 아님",
            ));
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir)?;
    Ok(())
}

/// 같은 디렉터리에 임시 이름으로 새로 만들고(심볼릭 링크를 따라가지 않음) 다 쓴 뒤 rename
fn write_new_file(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), SnapshotError> {
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let _ = fs::remove_file(&tmp);
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    if let Err(e) = fs::rename(&tmp, dir.join(name)) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// 현재가 아닌 옛 세대 중 오래된 것을 지운다 — 실패(Windows 에서 아직 매핑 중)는 다음에 다시
fn cleanup_old(dir: &Path, current: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let stale_snap = name.starts_with("files.") && name.ends_with(".snap") && name != current;
        let stale_tmp = name.starts_with(".files.") && name.contains(".tmp-");
        if !(stale_snap || stale_tmp) {
            continue;
        }
        let old_enough = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age.as_secs() >= KEEP_OLD_SECS);
        if old_enough {
            let _ = fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::table::Kind;
    use super::*;
    use std::ffi::OsStr;

    fn sample() -> TableBuilder {
        let mut b = TableBuilder::new();
        let root = b.add_root(RootInfo {
            path: OsStr::new("/office").as_encoded_bytes().to_vec(),
            mtime: 1,
            mtime_reliable: true,
        });
        let y = b.push(OsStr::new("2026년"), NO_PARENT, root, Kind::Dir, 1, 5);
        b.push(OsStr::new("보고서.hwp"), y, root, Kind::File, 2, 6);
        b
    }

    #[test]
    fn 쓰고_열면_같은_표가_나오고_세대가_오른다() {
        let tmp = tempfile::tempdir().unwrap();
        let b = sample();
        assert_eq!(publish(tmp.path(), &b).unwrap(), 1);
        assert_eq!(publish(tmp.path(), &b).unwrap(), 2);
        let s = Snapshot::open_current(tmp.path()).unwrap();
        assert_eq!(s.generation(), 2);
        s.verify_data().unwrap();
        let t = s.table();
        assert_eq!(t.len(), 2);
        assert_eq!(t.folded_name(1), "보고서.hwp".as_bytes());
        assert_eq!(
            t.path(1),
            Path::new("/office").join("2026년").join("보고서.hwp")
        );
        assert_eq!(t.mtime, &[5, 6]);
        assert_eq!(t.roots, b.roots.as_slice());
    }

    #[test]
    fn 손상된_스냅숏은_열지_않는다() {
        let tmp = tempfile::tempdir().unwrap();
        let good = encode(&sample(), 1).unwrap();
        let path = tmp.path().join("x.snap");
        let mut cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            good[..good.len() / 2].to_vec(),
            good[..DATA_START - 1].to_vec(),
        ];
        // 머리 한 바이트씩 뒤집기
        for i in [0, 8, 12, 20, 24, 32, 48, HEADER_LEN + 3, HEADER_LEN + 9] {
            let mut bad = good.clone();
            bad[i] ^= 0xff;
            cases.push(bad);
        }
        for bad in cases {
            fs::write(&path, &bad).unwrap();
            assert!(Snapshot::open(&path).is_err());
        }
        fs::write(&path, &good).unwrap();
        assert!(Snapshot::open(&path).is_ok());
    }

    #[test]
    fn 부모가_자식보다_뒤면_거절() {
        let mut b = sample();
        b.parent[0] = 1; // 2026년 의 부모가 자기 자식
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("x.snap");
        fs::write(&path, encode(&b, 1).unwrap()).unwrap();
        assert!(matches!(
            Snapshot::open(&path),
            Err(SnapshotError::Invalid("부모 순서"))
        ));
    }

    #[test]
    fn current_파일이_경로를_가리키면_거절() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(CURRENT_FILE), "../../etc/passwd").unwrap();
        assert!(Snapshot::open_current(tmp.path()).is_err());
    }
}
