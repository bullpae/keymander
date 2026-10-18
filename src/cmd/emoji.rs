//! `kmd emoji <query>` — search and copy emoji from CLI

use color_eyre::Result;
use kmd_core::plugin::builtin_emoji::EmojiExtension;

pub fn run(query: &str, list: bool, json: bool, copy_nth: Option<usize>) -> Result<()> {
    let ext = EmojiExtension;
    let results = ext.search_emoji(query);

    if results.is_empty() {
        if json {
            println!("[]");
        } else {
            eprintln!("No emoji found for \"{}\"", query);
        }
        return Ok(());
    }

    if json {
        // JSON output — parse name parts from item.name
        // Format: "😀 grinning face (활짝 웃는 얼굴)" or "😀 grinning face"
        let items: Vec<serde_json::Value> = results
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let full = item
                    .name
                    .strip_prefix(&item.path)
                    .unwrap_or(&item.name)
                    .trim();
                // Split English/Korean: "grinning face (활짝 웃는 얼굴)"
                let (en_name, ko_name) = if let Some(p) = full.rfind(" (") {
                    let en = full[..p].trim();
                    let ko = full[p + 2..].trim_end_matches(')').trim();
                    (en, ko)
                } else {
                    (full, "")
                };
                serde_json::json!({
                    "index": i + 1,
                    "emoji": item.path,
                    "name": en_name,
                    "name_ko": ko_name,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    // Determine which emoji to copy
    let copy_index = if list {
        None // --list: don't copy anything
    } else {
        Some(copy_nth.unwrap_or(1)) // default: copy first result
    };

    // Print results
    for (i, item) in results.iter().enumerate() {
        let marker = if copy_index == Some(i + 1) {
            " ←"
        } else {
            ""
        };
        println!(
            "  {:>2}. {} {}{}",
            i + 1,
            item.path, // the emoji character
            item.name
                .strip_prefix(&format!("{} ", item.path))
                .unwrap_or(&item.name),
            marker,
        );
    }

    // Copy to clipboard
    if let Some(idx) = copy_index {
        if idx >= 1 && idx <= results.len() {
            let emoji = &results[idx - 1].path;
            match kmd_core::clipboard::set_text(emoji) {
                Ok(()) => {
                    println!("\n  ✅ Copied: {}", emoji);
                }
                Err(e) => {
                    eprintln!("\n  ❌ Failed to copy: {}", e);
                }
            }
        } else {
            eprintln!("\n  ❌ Index {} out of range (1-{})", idx, results.len());
        }
    }

    Ok(())
}
