//! 설정 파일 저장 — 주석 보존 병합, 첫 실행 시드, 원자적 쓰기

use super::*;

/// 직렬화 결과(`fresh`)를 기존 문서(`existing`)에 병합해 주석·키 순서를 살린다.
///
/// 기존 문서를 기준으로 값만 갈아끼우므로 키에 붙은 주석(decor)이 그대로 남는다.
/// 기존 문서가 깨져서 파싱되지 않으면 병합을 포기하고 새 직렬화 결과를 쓴다 —
/// 저장 자체를 실패시키는 것보다 낫다.
pub(super) fn merge_into_document(existing: &str, fresh: &str) -> Result<String, ConfigError> {
    use toml_edit::DocumentMut;

    let Ok(mut doc) = existing.parse::<DocumentMut>() else {
        return Ok(fresh.to_string());
    };
    let new_doc = fresh
        .parse::<DocumentMut>()
        .map_err(|e| ConfigError::Serialize(e.to_string()))?;

    merge_table(doc.as_table_mut(), new_doc.as_table());
    Ok(doc.to_string())
}

/// `new`의 내용을 `old`에 재귀적으로 반영한다.
///
/// - 양쪽에 다 있는 하위 테이블은 재귀 (그 안의 주석도 보존)
/// - 값이 바뀌면 값만 교체하고 기존 decor(주석·공백)를 되돌려 붙인다
/// - `new`에 없는 키는 제거한다 (설정에서 사라진 항목)
fn merge_table(old: &mut toml_edit::Table, new: &toml_edit::Table) {
    let stale: Vec<String> = old
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !new.contains_key(k))
        .collect();
    for key in stale {
        old.remove(&key);
    }

    for (key, new_item) in new.iter() {
        match (old.get_mut(key), new_item) {
            // 양쪽 다 테이블 → 재귀해서 안쪽 주석까지 보존
            (Some(toml_edit::Item::Table(old_t)), toml_edit::Item::Table(new_t)) => {
                merge_table(old_t, new_t);
            }
            // 기존 키가 있으면 값만 교체하고 decor(주석/공백)를 복원
            (Some(old_item), _) => {
                let decor = old_item.as_value().map(|v| v.decor().clone());
                *old_item = new_item.clone();
                if let (Some(d), Some(v)) = (decor, old_item.as_value_mut()) {
                    *v.decor_mut() = d;
                }
            }
            // 새 키
            (None, _) => {
                old.insert(key, new_item.clone());
            }
        }
    }
}

/// 배포본이 exe 옆에 두는 첫 실행용 설정 (플랫폼 키맵 프리셋 포함).
pub const SEED_CONFIG_FILENAME: &str = "config.default.toml";

/// 설정 파일이 아직 없으면 exe 옆의 [`SEED_CONFIG_FILENAME`]을 복사해 둔다.
///
/// winget 설치본은 데이터를 패키지 폴더(`kmd-data\`)가 아닌 표준 위치에 둔다 —
/// winget은 업그레이드 때 패키지 폴더를 통째로 지워서 설정·이력이 사라졌다.
/// 그런데 표준 위치에서 설정 없이 시작하면 `Config::default()`라 포터블 zip이
/// 주던 mouse/nav 레이어 프리셋이 빠진다. 그 프리셋을 처음 한 번만 심는다.
/// 이후로는 사용자의 파일이므로 다시 덮어쓰지 않는다. 실패해도 기본값으로
/// 계속 동작하면 되므로 오류는 삼킨다.
pub(super) fn seed_config(config_path: &Path, exe_dir: &Path) {
    let seed = exe_dir.join(SEED_CONFIG_FILENAME);
    let Ok(content) = std::fs::read_to_string(&seed) else {
        return;
    };
    if let Some(parent) = config_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = write_atomic(config_path, &content);
}

pub(super) fn write_atomic(path: &Path, content: &str) -> Result<(), ConfigError> {
    crate::fsutil::write_atomic(path, content.as_bytes(), false)
        .map_err(|e| ConfigError::Io(path.to_path_buf(), e))
}
