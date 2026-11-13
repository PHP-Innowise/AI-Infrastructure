use crate::{Result, common};
use serde_json::Value;
use std::{collections::HashMap, path::Path};
fn list<'a>(h: &'a Value, key: &str, fallback: &str) -> &'a [Value] {
    h.get(key)
        .or_else(|| h.get(fallback))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn aggregate(results: &[Value]) -> (u64, u64) {
    let (mut correct, mut total) = (0, 0);
    for r in results {
        let runs = r["runs"].as_u64().unwrap_or(0);
        let triggers = r["triggers"].as_u64().unwrap_or(0);
        total += runs;
        correct += if r["should_trigger"].as_bool().unwrap_or(true) {
            triggers
        } else {
            runs.saturating_sub(triggers)
        };
    }
    (correct, total)
}
fn score_class(correct: u64, total: u64) -> &'static str {
    if total > 0 {
        let ratio = correct as f64 / total as f64;
        if ratio >= 0.8 {
            return "score-good";
        }
        if ratio >= 0.5 {
            return "score-ok";
        }
    }
    "score-bad"
}
fn value(v: &Value, key: &str, default: &str) -> String {
    v.get(key)
        .map(common::display)
        .unwrap_or_else(|| default.into())
}
fn results_by_query(results: &[Value]) -> HashMap<&Value, &Value> {
    results.iter().map(|r| (&r["query"], r)).collect()
}
pub fn generate_html(data: &Value, auto_refresh: bool, name: &str) -> String {
    let history = data["history"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let empty = Value::Null;
    let first = history.first().unwrap_or(&empty);
    let train = list(first, "train_results", "results");
    let test = list(first, "test_results", "test_results");
    let prefix = if name.is_empty() {
        String::new()
    } else {
        common::html(&format!("{name} — "))
    };
    let refresh = if auto_refresh {
        "<meta http-equiv=\"refresh\" content=\"5\">\n"
    } else {
        ""
    };
    let mut out = format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">{refresh}<title>{prefix}Skill Description Optimization</title><style>{}</style></head><body><h1>{prefix}Skill Description Optimization</h1>\n",
        include_str!("../resources/report.css")
    );
    out.push_str("<div class=\"explainer\"><strong>Optimizing your skill's description.</strong> Each row is a measured description attempt. Green checks show cases that passed their trigger threshold; red crosses show cases that failed. Train observations inform the next description. Held-out test observations select the reported best description and are withheld from the improvement prompt. These trigger measurements do not establish task benefit or modify SKILL.md.</div>\n");
    out.push_str(&format!("<div class=\"summary\"><p><strong>Original:</strong> {}</p><p class=\"best\"><strong>Best:</strong> {}</p><p><strong>Best Score:</strong> {} {}</p><p><strong>Iterations:</strong> {} | <strong>Train:</strong> {} | <strong>Test:</strong> {}</p></div>\n",common::html(&value(data,"original_description","N/A")),common::html(&value(data,"best_description","N/A")),common::html(&value(data,"best_score","N/A")),if data["best_test_score"].is_string(){"(test)"}else{"(train)"},common::html(&value(data,"iterations_run","0")),common::html(&value(data,"train_size","?")),common::html(&value(data,"test_size","?"))));
    out.push_str("<div class=\"legend\"><span>Query columns:</span><span class=\"legend-item\"><span class=\"legend-swatch swatch-positive\"></span>Should trigger</span><span class=\"legend-item\"><span class=\"legend-swatch swatch-negative\"></span>Should NOT trigger</span><span class=\"legend-item\"><span class=\"legend-swatch swatch-train\"></span>Train</span><span class=\"legend-item\"><span class=\"legend-swatch swatch-test\"></span>Test</span></div><div class=\"table-container\"><table><thead><tr><th>Iter</th><th>Train</th><th>Test</th><th class=\"query-col\">Description</th>");
    for (is_test, queries) in [(false, train), (true, test)] {
        for q in queries {
            out.push_str(&format!(
                "<th class=\"{}{}\">{}</th>",
                if is_test { "test-col " } else { "" },
                if q["should_trigger"].as_bool().unwrap_or(true) {
                    "positive-col"
                } else {
                    "negative-col"
                },
                common::html(q["query"].as_str().unwrap_or(""))
            ));
        }
    }
    out.push_str("</tr></thead><tbody>");
    let field = if test.is_empty() {
        "train_passed"
    } else {
        "test_passed"
    };
    let mut best = None;
    let mut score = 0;
    for (index, h) in history.iter().enumerate() {
        let current = h
            .get(field)
            .or_else(|| h.get("passed"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if best.is_none() || current > score {
            best = Some(index);
            score = current;
        }
    }
    for (index, h) in history.iter().enumerate() {
        let a = list(h, "train_results", "results");
        let b = list(h, "test_results", "test_results");
        let (ac, at) = aggregate(a);
        let (bc, bt) = aggregate(b);
        out.push_str(&format!("<tr class=\"{}\"><td>{}</td><td><span class=\"score {}\">{ac}/{at}</span></td><td><span class=\"score {}\">{bc}/{bt}</span></td><td class=\"description\">{}</td>",if best==Some(index){"best-row"}else{""},common::html(&value(h,"iteration","?")),score_class(ac,at),score_class(bc,bt),common::html(h["description"].as_str().unwrap_or(""))));
        for (is_test, queries, results) in [(false, train, a), (true, test, b)] {
            let results = results_by_query(results);
            for q in queries {
                let result = results.get(&q["query"]).copied();
                let passed = result.is_some_and(|r| r["pass"] == true);
                out.push_str(&format!(
                    "<td class=\"result {}{}\">{}<span class=\"rate\">{}/{}</span></td>",
                    if is_test { "test-result " } else { "" },
                    if passed { "pass" } else { "fail" },
                    if passed { "✓" } else { "✗" },
                    result.and_then(|r| r["triggers"].as_u64()).unwrap_or(0),
                    result.and_then(|r| r["runs"].as_u64()).unwrap_or(0)
                ));
            }
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table></div></body></html>\n");
    out
}
pub fn execute(v: &Value) -> Result<Value> {
    let input = common::positional(v, "input", 0)?;
    let data = if input == "-" {
        common::stdin_json()?
    } else {
        common::json_file(Path::new(&input))?
    };
    let html = generate_html(
        &data,
        common::flag(v, "auto_refresh"),
        common::text(v, "skill_name").unwrap_or(""),
    );
    if let Some(path) = common::text(v, "output") {
        std::fs::write(path, &html)?;
        Ok(common::cli(
            0,
            "",
            format!("Report written to {path}\n"),
            serde_json::json!({"output":path}),
        ))
    } else {
        Ok(common::cli(0, html, "", Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn duplicate_queries_display_the_last_train_and_test_results() {
        let data = json!({"history":[{
            "train_results":[
                {"query":"train","pass":false,"triggers":1,"runs":9},
                {"query":"train","pass":true,"triggers":7,"runs":11}
            ],
            "test_results":[
                {"query":"test","pass":true,"triggers":9,"runs":10},
                {"query":"test","pass":false,"triggers":2,"runs":3}
            ]
        }]});
        let html = generate_html(&data, false, "");
        assert_eq!(
            html.matches("<td class=\"result pass\">✓<span class=\"rate\">7/11</span></td>")
                .count(),
            2
        );
        assert_eq!(
            html.matches(
                "<td class=\"result test-result fail\">✗<span class=\"rate\">2/3</span></td>"
            )
            .count(),
            2
        );
        assert!(!html.contains("class=\"rate\">1/9"));
        assert!(!html.contains("class=\"rate\">9/10"));
    }

    #[test]
    fn result_lookup_preserves_json_equality_missing_values_and_column_order() {
        let data = json!({"history":[
            {"train_results":[
                {"query":"missing"},
                {"query":null},
                {"query":{"a":1,"b":[2]}},
                {"query":1},
                {"query":"1"}
            ]},
            {"train_results":[
                {"query":null,"pass":false,"triggers":2,"runs":3},
                {"pass":true,"triggers":5,"runs":6},
                {"query":{"b":[2],"a":1},"pass":true,"triggers":7,"runs":8},
                {"query":1,"pass":true,"triggers":9,"runs":10},
                {"query":"1","pass":false,"triggers":11,"runs":12}
            ]}
        ]});
        let html = generate_html(&data, false, "");
        let expected = concat!(
            "<td class=\"result fail\">✗<span class=\"rate\">0/0</span></td>",
            "<td class=\"result pass\">✓<span class=\"rate\">5/6</span></td>",
            "<td class=\"result pass\">✓<span class=\"rate\">7/8</span></td>",
            "<td class=\"result pass\">✓<span class=\"rate\">9/10</span></td>",
            "<td class=\"result fail\">✗<span class=\"rate\">11/12</span></td>"
        );
        assert!(html.contains(expected), "query columns changed: {html}");
    }

    #[test]
    fn query_headers_keep_source_order_and_html_escaping() {
        let data = json!({"history":[{"train_results":[
            {"query":"<b>&\"","pass":true,"triggers":1,"runs":1},
            {"query":"after","pass":false,"triggers":0,"runs":1}
        ]}]});
        let html = generate_html(&data, false, "<name>");
        let escaped = html.find("&lt;b&gt;&amp;&quot;").expect("escaped query");
        let after = html.find(">after</th>").expect("second query");
        assert!(escaped < after);
        assert!(html.contains("&lt;name&gt;"));
        assert!(!html.contains("<b>&\""));
    }
}
