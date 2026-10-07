//! 소스 감사 테스트: Windows에서 의도치 않은 콘솔 창 노출 방지
//!
//! 기존 테스트는 `kmd-core/src` 파일 단위 문자열 검색이어서 회귀 탐지 범위가 좁았다.
//! 이 테스트는 워크스페이스 핵심 소스를 대상으로, `Command::new` "호출 단위"로
//! `creation_flags(CREATE_NO_WINDOW|DETACHED_PROCESS)` 적용 여부를 점검한다.

use std::path::{Path, PathBuf};

/// 면제 파일: 콘솔 창이 의도된 상호작용 시나리오
const EXEMPT_FILES: &[&str] = &[
    "builtin_shell.rs", // 사용자가 직접 실행하는 셸 명령
    "app.rs",           // launch_in_terminal()는 의도적으로 새 콘솔 창을 연다
    "config.rs",        // CLI에서 편집기 실행은 콘솔 숨김 대상이 아님
    "macos.rs",         // macOS 전용 백엔드 파일
];

const EXEMPT_PATH_SUFFIXES: &[&str] = &[
    "/src/cmd/daemon.rs",  // CLI 맥락의 보조 명령 실행
    "/src/cmd/upgrade.rs", // winget 진행 상황을 보여주려고 의도적으로 새 콘솔 창을 연다
];

const COMMAND_LOOKAHEAD_LINES: usize = 24;

#[test]
fn test_windows_command_calls_use_hidden_flags() {
    let workspace_root = workspace_root();
    let scan_roots = [
        workspace_root.join("crates/kmd-core/src"),
        workspace_root.join("crates/kmd-daemon/src"),
        workspace_root.join("crates/kmd-desktop/src"),
        workspace_root.join("src"),
    ];

    let mut violations = Vec::new();

    for root in scan_roots {
        if !root.is_dir() {
            continue;
        }
        visit_rs_files(&root, &mut |path| {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if EXEMPT_FILES.contains(&file_name) {
                return;
            }
            let path_s = path.to_string_lossy().replace('\\', "/");
            if EXEMPT_PATH_SUFFIXES
                .iter()
                .any(|suffix| path_s.ends_with(suffix))
            {
                return;
            }

            let Ok(content) = std::fs::read_to_string(path) else {
                return;
            };
            if !content.contains("Command::new(") {
                return;
            }

            let code = strip_test_modules(&content);
            let lines: Vec<&str> = code.lines().collect();
            let command_calls = command_call_lines(&lines);
            if command_calls.is_empty() {
                return;
            }

            for line_idx in command_calls {
                if is_non_windows_scope(&lines, line_idx) {
                    continue;
                }
                if is_known_non_windows_command(&lines, line_idx) {
                    continue;
                }
                if !has_hidden_flag_near_call(&lines, line_idx) {
                    violations.push(format!(
                        "{}:{} Command::new 호출에 Windows hidden creation_flags가 없습니다",
                        path.display(),
                        line_idx + 1
                    ));
                }
            }
        });
    }

    if !violations.is_empty() {
        panic!(
            "Windows 콘솔 창 노출 위험 호출 발견 ({}):\n  - {}",
            violations.len(),
            violations.join("\n  - ")
        );
    }
}

fn workspace_root() -> PathBuf {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_root
        .parent()
        .and_then(Path::parent)
        .unwrap_or(crate_root.as_path())
        .to_path_buf()
}

/// .rs 파일을 재귀적으로 방문
fn visit_rs_files(dir: &Path, f: &mut dyn FnMut(&Path)) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            visit_rs_files(&path, f);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            f(&path);
        }
    }
}

/// 테스트 모듈 블록(`#[cfg(test)] mod ...`)은 감사 대상에서 제외
fn strip_test_modules(content: &str) -> String {
    let mut out = String::new();
    let mut skip_next_mod = false;
    let mut skipping = false;
    let mut depth = 0i32;

    for line in content.lines() {
        let t = line.trim();
        if !skipping && t == "#[cfg(test)]" {
            skip_next_mod = true;
            continue;
        }
        if skip_next_mod && t.starts_with("mod ") && t.contains('{') {
            skipping = true;
            skip_next_mod = false;
            depth = brace_delta(t);
            continue;
        }
        if skipping {
            depth += brace_delta(t);
            if depth <= 0 {
                skipping = false;
                depth = 0;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn brace_delta(s: &str) -> i32 {
    s.chars().fold(0, |acc, ch| match ch {
        '{' => acc + 1,
        '}' => acc - 1,
        _ => acc,
    })
}

fn command_call_lines(lines: &[&str]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            let t = line.trim();
            if t.contains("Command::new(") || t.contains("std::process::Command::new(") {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

/// 단순 cfg 스코프 추적으로 non-windows 블록 내부 호출을 제외
fn is_non_windows_scope(lines: &[&str], target_idx: usize) -> bool {
    let mut depth = 0i32;
    let mut non_windows_blocks: Vec<i32> = Vec::new();
    let mut pending_non_windows = false;

    for (idx, line) in lines.iter().enumerate() {
        while non_windows_blocks.last().is_some_and(|d| depth <= *d) {
            non_windows_blocks.pop();
        }

        let t = line.trim();
        if t.starts_with("#[cfg(")
            && (t.contains("target_os = \"macos\"")
                || t.contains("target_os = \"linux\"")
                || t.contains("not(target_os = \"windows\")")
                || t.contains("not(windows)")
                || t.contains("cfg(unix)")
                || t.contains("not(any(target_os = \"windows\""))
        {
            pending_non_windows = true;
        }

        if pending_non_windows && t.contains('{') {
            non_windows_blocks.push(depth);
            pending_non_windows = false;
        }

        if idx == target_idx {
            return !non_windows_blocks.is_empty();
        }

        depth += brace_delta(t);
    }

    false
}

fn has_hidden_flag_near_call(lines: &[&str], start: usize) -> bool {
    // 패턴 1: 체이닝 호출 (Command::new(...).creation_flags(...).spawn())
    if has_hidden_flag_in_chain(lines, start) {
        return true;
    }

    // 패턴 2: 변수에 바인딩 후 creation_flags 적용
    if let Some(var_name) = assigned_command_var(lines[start]) {
        return has_hidden_flag_for_var(lines, start, &var_name);
    }

    false
}

fn has_hidden_flag_in_chain(lines: &[&str], start: usize) -> bool {
    let end = (start + COMMAND_LOOKAHEAD_LINES).min(lines.len().saturating_sub(1));
    let mut saw_creation_flags = false;
    let mut saw_expected_flag = false;

    for line in lines.iter().take(end + 1).skip(start) {
        let t = line.trim();
        if t.contains("creation_flags(") {
            saw_creation_flags = true;
        }
        if t.contains("CREATE_NO_WINDOW") || t.contains("DETACHED_PROCESS") {
            saw_expected_flag = true;
        }
        if t.ends_with(';') {
            break;
        }
    }

    saw_creation_flags && saw_expected_flag
}

fn assigned_command_var(line: &str) -> Option<String> {
    if !line.contains("Command::new(") || !line.contains("let ") || !line.contains('=') {
        return None;
    }
    let lhs = line.split('=').next()?.trim();
    let name = lhs
        .trim_start_matches("let ")
        .trim_start_matches("mut ")
        .split_whitespace()
        .last()?;
    if name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        Some(name.to_string())
    } else {
        None
    }
}

fn has_hidden_flag_for_var(lines: &[&str], start: usize, var_name: &str) -> bool {
    let end = (start + COMMAND_LOOKAHEAD_LINES).min(lines.len().saturating_sub(1));
    let call = format!("{var_name}.creation_flags(");
    let mut saw_creation_flags = false;
    let mut saw_expected_flag = false;

    for line in lines.iter().take(end + 1).skip(start + 1) {
        let t = line.trim();
        if t.contains(&call) {
            saw_creation_flags = true;
        }
        if t.contains("CREATE_NO_WINDOW") || t.contains("DETACHED_PROCESS") {
            saw_expected_flag = true;
        }
    }

    saw_creation_flags && saw_expected_flag
}

/// 코드 상 non-windows 전용 명령으로 명확히 식별 가능한 경우 감사 제외
fn is_known_non_windows_command(lines: &[&str], idx: usize) -> bool {
    let line = lines[idx];
    if line.contains("Command::new(\"mdfind\")") {
        return true;
    }

    // `let cmd = if which(\"plocate\") ...` 형태(collect_locate) 처리
    let back_start = idx.saturating_sub(16);
    lines
        .iter()
        .take(idx + 1)
        .skip(back_start)
        .any(|l| l.contains("let cmd = if which(\"plocate\")") || l.contains("which(\"locate\")"))
}
