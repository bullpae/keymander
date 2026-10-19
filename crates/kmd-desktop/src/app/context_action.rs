//! 상세 패널 컨텍스트 액션 — 종류별 액션 목록·레이블·단축키

use super::*;

#[derive(Debug, Clone, Copy)]
pub enum ContextAction {
    Open,
    OpenAsAdmin,
    OpenFolder,
    CopyPath,
    CopyName,
}

impl ContextAction {
    fn label(&self) -> &str {
        match self {
            Self::Open => "열기",
            Self::OpenAsAdmin => "관리자 권한으로 열기",
            Self::OpenFolder => "파일 위치 열기",
            Self::CopyPath => "경로 복사",
            Self::CopyName => "이름 복사",
        }
    }

    /// Calculator/Emoji 컨텍스트에서 보여줄 레이블 (값 복사)
    pub(super) fn label_for_kind(&self, kind: ItemKind) -> &str {
        match (self, kind) {
            (Self::CopyName, ItemKind::Calculator) => "값 복사",
            (Self::CopyName, ItemKind::Emoji) => "이모지 복사",
            (Self::Open, ItemKind::Calculator) => "복사 후 닫기",
            _ => self.label(),
        }
    }

    pub(super) fn shortcut(&self) -> &str {
        match self {
            Self::Open => "Ctrl+1",
            Self::OpenAsAdmin => "Ctrl+Shift+Enter",
            Self::OpenFolder => "Ctrl+2",
            Self::CopyPath => "Ctrl+3",
            Self::CopyName => "Ctrl+4",
        }
    }

    /// 이 액션에 할당된 Ctrl+N 인덱스 (0-based). 위치가 아닌 타입으로 결정.
    /// Calculator처럼 액션이 1개뿐이어도 CopyName은 항상 Ctrl+4(=3)로 동작.
    pub(super) fn shortcut_index(&self) -> usize {
        match self {
            Self::Open => 0,
            Self::OpenFolder => 1,
            Self::CopyPath => 2,
            Self::CopyName => 3,
            Self::OpenAsAdmin => usize::MAX,
        }
    }
}

/// 아이템 종류에 따라 사용 가능한 컨텍스트 액션 목록
pub(super) fn context_actions_for(kind: ItemKind) -> Vec<ContextAction> {
    match kind {
        ItemKind::App | ItemKind::Executable => vec![
            ContextAction::Open,
            ContextAction::OpenAsAdmin,
            ContextAction::OpenFolder,
            ContextAction::CopyPath,
        ],
        ItemKind::File => vec![
            ContextAction::Open,
            ContextAction::OpenFolder,
            ContextAction::CopyPath,
            ContextAction::CopyName,
        ],
        ItemKind::Directory => vec![ContextAction::Open, ContextAction::CopyPath],
        ItemKind::WebSearch => vec![ContextAction::Open, ContextAction::CopyName],
        ItemKind::Shell => vec![ContextAction::Open, ContextAction::CopyName],
        ItemKind::SystemCommand => vec![ContextAction::Open],
        ItemKind::Calculator | ItemKind::Emoji => vec![ContextAction::CopyName],
    }
}
