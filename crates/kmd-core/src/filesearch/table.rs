//! 구조 배열 색인 — 항목을 객체 대신 열(column) 배열로 둔다 (docs/22 §3.3)
//!
//! 쓰는 쪽(스캐너)은 [`TableBuilder`]에 `Vec` 으로 쌓고, 읽는 쪽(질의)은 [`Table`]
//! — 같은 열을 빌린 슬라이스 묶음 — 만 본다. 스냅숏을 mmap 한 바이트 위에서도
//! 같은 [`Table`] 을 만들 수 있어 질의 코드가 하나다.
//!
//! 규칙:
//! - **원본 이름(`raw`)과 검색용 이름(`folded`)을 분리**한다. 접으면(NFC·소문자) 길이가
//!   바뀌고, 접은 이름으로는 ext4·SMB 의 NFD 이름이나 Windows 의 짝 없는 서로게이트를
//!   되살릴 수 없다. 경로 복원은 `raw` 로만 한다.
//! - **부모가 자식보다 먼저 온다** (`parent[row] < row`). 걷기 순서가 그렇고, 조상 정보를
//!   앞에서 뒤로 한 번에 전파하는 질의(폴더 마스크)가 이 순서에 기댄다.
//! - 전체 경로는 저장하지 않는다 — 결과로 보여 줄 행만 `parent` 를 따라 복원한다.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 부모가 없음 — 검색 폴더 바로 아래 항목
pub const NO_PARENT: u32 = u32::MAX;

/// 항목 종류 (`meta` 하위 2비트)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File = 0,
    Dir = 1,
}

const KIND_MASK: u32 = 0b11;
const DEPTH_SHIFT: u32 = 2;
const DEPTH_MASK: u32 = 0xff;

/// 검색 폴더 하나 — 스냅숏에 경로·스캔 시각과 함께 저장된다
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootInfo {
    /// 원본 경로 바이트 ([`OsStr::as_encoded_bytes`])
    pub path: Vec<u8>,
    /// 검색 폴더 자신의 수정 시각 (열 때 대조용, ns)
    pub mtime: i64,
    /// 폴더 수정 시각이 하위 변경을 믿을 만하게 반영하는 볼륨인지 (FAT·네트워크는 false)
    pub mtime_reliable: bool,
}

impl RootInfo {
    pub fn path_buf(&self) -> PathBuf {
        bytes_to_path(&self.path)
    }
}

/// 질의가 보는 열 묶음 — 메모리(`TableBuilder`)든 매핑된 스냅숏이든 같은 모양
#[derive(Clone, Copy)]
pub struct Table<'a> {
    pub raw: &'a [u8],
    pub raw_off: &'a [u32],
    pub folded: &'a [u8],
    pub folded_off: &'a [u32],
    pub parent: &'a [u32],
    pub root_id: &'a [u16],
    pub bag: &'a [u64],
    pub meta: &'a [u32],
    pub mtime: &'a [i64],
    pub roots: &'a [RootInfo],
}

impl<'a> Table<'a> {
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    pub fn raw_name(&self, row: usize) -> &'a [u8] {
        &self.raw[self.raw_off[row] as usize..self.raw_off[row + 1] as usize]
    }

    /// 검색용 이름 — 이름 사이 구분 바이트(0)는 빼고 돌려준다
    pub fn folded_name(&self, row: usize) -> &'a [u8] {
        let end = self.folded_off[row + 1] as usize - 1;
        &self.folded[self.folded_off[row] as usize..end]
    }

    pub fn kind(&self, row: usize) -> Kind {
        if self.meta[row] & KIND_MASK == Kind::Dir as u32 {
            Kind::Dir
        } else {
            Kind::File
        }
    }

    /// 검색 폴더 아래 몇 단계인지 (바로 아래 = 1)
    pub fn depth(&self, row: usize) -> u32 {
        (self.meta[row] >> DEPTH_SHIFT) & DEPTH_MASK
    }

    /// 전체 경로 복원 — 검색 폴더 경로 + 조상 이름들 + 자기 이름
    pub fn path(&self, row: usize) -> PathBuf {
        let mut chain = vec![row];
        let mut cur = self.parent[row];
        while cur != NO_PARENT {
            chain.push(cur as usize);
            cur = self.parent[cur as usize];
        }
        let mut path = self.roots[self.root_id[row] as usize].path_buf();
        for &r in chain.iter().rev() {
            path.push(bytes_to_path(self.raw_name(r)));
        }
        path
    }
}

/// 스캐너가 쌓는 쪽
#[derive(Debug, Default, Clone)]
pub struct TableBuilder {
    pub raw: Vec<u8>,
    pub raw_off: Vec<u32>,
    pub folded: Vec<u8>,
    pub folded_off: Vec<u32>,
    pub parent: Vec<u32>,
    pub root_id: Vec<u16>,
    pub bag: Vec<u64>,
    pub meta: Vec<u32>,
    pub mtime: Vec<i64>,
    pub roots: Vec<RootInfo>,
}

impl TableBuilder {
    pub fn new() -> Self {
        Self {
            raw_off: vec![0],
            folded_off: vec![0],
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    pub fn add_root(&mut self, root: RootInfo) -> u16 {
        self.roots.push(root);
        (self.roots.len() - 1) as u16
    }

    /// 항목 하나를 붙이고 행 번호를 돌려준다. `parent` 는 이미 붙은 행이어야 한다.
    pub fn push(
        &mut self,
        name: &OsStr,
        parent: u32,
        root: u16,
        kind: Kind,
        depth: u32,
        mtime: i64,
    ) -> u32 {
        let row = self.parent.len() as u32;
        debug_assert!(parent == NO_PARENT || parent < row, "부모가 먼저 와야 한다");
        self.raw.extend_from_slice(name.as_encoded_bytes());
        self.raw_off.push(self.raw.len() as u32);

        let folded = fold(&name.to_string_lossy());
        self.bag.push(char_bag(&folded));
        self.folded.extend_from_slice(folded.as_bytes());
        self.folded.push(0);
        self.folded_off.push(self.folded.len() as u32);

        self.parent.push(parent);
        self.root_id.push(root);
        self.meta
            .push(kind as u32 | (depth.min(DEPTH_MASK) << DEPTH_SHIFT));
        self.mtime.push(mtime);
        row
    }

    /// 다른 표의 행을 이름을 다시 접지 않고 그대로 옮긴다 (열 때 대조가 새 표를 만들 때)
    pub fn push_copied(&mut self, t: &Table<'_>, row: usize, parent: u32, mtime: i64) -> u32 {
        let new_row = self.parent.len() as u32;
        debug_assert!(parent == NO_PARENT || parent < new_row);
        self.raw.extend_from_slice(t.raw_name(row));
        self.raw_off.push(self.raw.len() as u32);
        self.folded.extend_from_slice(t.folded_name(row));
        self.folded.push(0);
        self.folded_off.push(self.folded.len() as u32);
        self.bag.push(t.bag[row]);
        self.parent.push(parent);
        self.root_id.push(t.root_id[row]);
        self.meta.push(t.meta[row]);
        self.mtime.push(mtime);
        new_row
    }

    pub fn table(&self) -> Table<'_> {
        Table {
            raw: &self.raw,
            raw_off: &self.raw_off,
            folded: &self.folded,
            folded_off: &self.folded_off,
            parent: &self.parent,
            root_id: &self.root_id,
            bag: &self.bag,
            meta: &self.meta,
            mtime: &self.mtime,
            roots: &self.roots,
        }
    }
}

/// 검색용으로 접기 — NFC + 소문자 (질의도 같은 함수로 접는다)
pub fn fold(s: &str) -> String {
    crate::textenc::nfc(s).to_lowercase()
}

/// 문자 가방 — 이름에 든 글자 종류를 64비트로. `q & bag == q` 가 아니면 매칭될 수 없다.
///
/// ASCII 영숫자 36비트, 한글 음절은 초성 19비트(음절이 들어 있으면 그 초성 비트도 선다),
/// 그 밖의 글자는 코드포인트를 9비트로 접어 넣는다. 필요조건만 보장하면 되므로
/// 서로 다른 글자가 같은 비트를 써도(거짓 양성) 괜찮다.
pub fn char_bag(folded: &str) -> u64 {
    folded.chars().fold(0u64, |bag, c| bag | char_bit(c))
}

fn char_bit(c: char) -> u64 {
    match c {
        'a'..='z' => 1 << (c as u32 - 'a' as u32),
        '0'..='9' => 1 << (26 + c as u32 - '0' as u32),
        '\u{AC00}'..='\u{D7A3}' => 1 << (36 + (c as u32 - 0xAC00) / 588),
        // 초성 자모는 그 음절들과 같은 비트 — 초성 질의(`ㅂㄱㅅ`)가 가방에서 걸러지지 않게
        c if crate::hangul::choseong_index(c).is_some() => {
            1 << (36 + crate::hangul::choseong_index(c).unwrap_or(0))
        }
        c if c.is_ascii() => 0, // 구분자·기호는 거르는 데 쓰지 않는다
        c => 1 << (55 + (c as u32 % 9)),
    }
}

/// [`OsStr::as_encoded_bytes`] 로 저장한 바이트를 경로로 되돌린다.
pub fn bytes_to_path(bytes: &[u8]) -> PathBuf {
    // SAFETY: 이 바이트는 같은 플랫폼에서 `as_encoded_bytes` 로 만든 것이다
    // (스냅숏 헤더의 플랫폼 표시로 다른 OS 의 스냅숏은 열지 않는다).
    let s = unsafe { OsStr::from_encoded_bytes_unchecked(bytes) };
    Path::new(s).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 경로는_원본_이름으로_복원하고_검색은_접은_이름으로() {
        let mut b = TableBuilder::new();
        let root = b.add_root(RootInfo {
            path: OsStr::new("/r").as_encoded_bytes().to_vec(),
            mtime: 0,
            mtime_reliable: true,
        });
        // macOS 식 NFD "보고서" (ㅂ+ㅗ ...) 와 대문자
        let nfd = "\u{1107}\u{1169}\u{1100}\u{1169}\u{1109}\u{1165}";
        let dir = b.push(OsStr::new("2026년"), NO_PARENT, root, Kind::Dir, 1, 0);
        let file = b.push(
            OsStr::new(&format!("{nfd}_Final.TXT")),
            dir,
            root,
            Kind::File,
            2,
            0,
        );
        let t = b.table();
        assert_eq!(t.len(), 2);
        assert_eq!(t.folded_name(file as usize), "보고서_final.txt".as_bytes());
        assert_eq!(
            t.path(file as usize),
            Path::new("/r")
                .join("2026년")
                .join(format!("{nfd}_Final.TXT")),
            "원본(NFD·대문자) 그대로"
        );
        assert_eq!(t.kind(dir as usize), Kind::Dir);
        assert_eq!(t.depth(file as usize), 2);
    }

    #[test]
    fn 문자_가방은_필요조건() {
        let name = char_bag(&fold("보고서_2026.hwp"));
        for q in ["2026", "hwp", "보고", "ㅂ"] {
            let qb = char_bag(&fold(q));
            if q == "ㅂ" {
                // 호환 자모 단독은 음절이 아니라 기타 비트 — 초성 검색은 F4
                continue;
            }
            assert_eq!(qb & name, qb, "{q}");
        }
        let z = char_bag("zzz");
        assert_ne!(z & name, z);
    }
}
