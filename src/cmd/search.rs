//! `kmd search <query>` — search from CLI

use color_eyre::Result;

pub fn run(query: &str, limit: usize, json: bool) -> Result<()> {
    let config = super::load_config()?;

    // 웹 프리픽스 질의 — 데스크톱/TUI와 같은 통합 분류기를 쓴다
    // (예전엔 parse_* 체인을 따로 들고 있던 세 번째 라우터였다).
    if run_web_query(query, &config, json)? {
        return Ok(());
    }

    let mut engine = super::create_search_engine(&config);

    let (mode, mut results) = engine.search(query, limit);

    // Apply history boost
    if let Ok(db) = super::open_db() {
        kmd_core::history::boost_results(&mut results, &db);
    }

    if json {
        let items: Vec<_> = results.iter().map(|r| &r.item).collect();
        let json_str = serde_json::to_string_pretty(&items)?;
        println!("{}", json_str);
    } else {
        println!(
            "Search: \"{}\" [{}] ({} results)\n",
            query,
            mode.label(),
            results.len()
        );
        for (i, result) in results.iter().enumerate() {
            println!(
                "  {:>2}. {} {:<30} [{}]  {}",
                i + 1,
                result.item.icon,
                result.item.name,
                result.item.kind,
                result.item.path,
            );
        }
    }

    Ok(())
}

fn print_items_json(items: &[kmd_core::index::IndexItem]) -> color_eyre::Result<()> {
    let json = serde_json::to_string_pretty(items)?;
    println!("{}", json);
    Ok(())
}

/// `@...` 웹 질의를 처리했으면 true. 서비스에 매칭되지 않는 `@xyz`는 일반 검색으로 넘긴다.
fn run_web_query(query: &str, config: &kmd_core::Config, json: bool) -> Result<bool> {
    use kmd_core::web::{self, WebQueryResult};

    let l = &config.launcher;
    let emoji = config.general.emoji_icons;
    // (빈 질의일 때 제목, 사용법 별칭 목록, 기본 별칭, 인자 이름, 질의, 항목)
    let (title, prefixes, fallback, arg, q, items) =
        match web::classify_web_query(query, &web::WebQueryConfig::from_launcher(l)) {
            WebQueryResult::Spell(q) => {
                let items = web::spell_result_items(&q, &l.spell_providers, emoji);
                (
                    "spelling providers",
                    &l.spell_prefixes,
                    "@sp",
                    "<text>",
                    q,
                    items,
                )
            }
            WebQueryResult::Translate(dir, q) => {
                let items = web::translate_result_items(&q, dir, &l.translate_providers, emoji);
                (
                    "translate providers",
                    &l.translate_prefixes,
                    "@tr",
                    "<text>",
                    q,
                    items,
                )
            }
            WebQueryResult::MultiLlm(_, q) => {
                let items = web::multi_llm_result_items(&q, &l.multi_llm_providers, emoji);
                (
                    "multi-LLM providers",
                    &l.multi_llm_prefixes,
                    "@llm",
                    "<prompt>",
                    q,
                    items,
                )
            }
            WebQueryResult::MultiWeb(_, q) => {
                let items = web::multi_web_result_items(&q, &l.multi_web_providers, emoji);
                (
                    "multi-web engines",
                    &l.multi_web_prefixes,
                    "@msearch",
                    "<query>",
                    q,
                    items,
                )
            }
            WebQueryResult::Single(_, q) if q.is_empty() => {
                let items = web::list_services_as_items("", emoji);
                if json {
                    print_items_json(&items)?;
                } else {
                    for item in &items {
                        println!("  {} {}", item.icon, item.name);
                    }
                }
                return Ok(true);
            }
            WebQueryResult::Single(service, q) => {
                if json {
                    print_items_json(&[web::search_result_item(service, &q, emoji)])?;
                } else {
                    let url = web::build_search_url(service, &q);
                    println!("  {} {} → {}", service.icon, service.name, url);
                }
                return Ok(true);
            }
            WebQueryResult::Browse(_) => return Ok(false),
        };

    if json {
        print_items_json(&items)?;
        return Ok(true);
    }
    if q.is_empty() {
        println!("Configured {title}:");
    }
    for item in &items {
        println!("  {} {}", item.icon, item.name);
    }
    if q.is_empty() {
        let usage = prefixes.first().map(String::as_str).unwrap_or(fallback);
        println!("\nUsage: {usage} {arg}");
    }
    Ok(true)
}
