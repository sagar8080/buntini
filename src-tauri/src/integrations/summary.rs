/// Render only complete, explicitly sourced model claims over the cards that
/// this approved request actually returned. Source numbers are data validated
/// here, rather than citation brackets supplied in unconstrained prose.
pub fn render_card_summary(raw: &str, card_count: usize) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Claim {
        text: String,
        sources: Vec<String>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Summary {
        claims: Vec<Claim>,
        coverage: String,
        missing: Vec<String>,
    }
    if raw.len() > 128 * 1024 || card_count == 0 || card_count > super::MAX_CARDS {
        return Err("The result summary has an invalid size or source set.".into());
    }
    let trimmed = raw.trim();
    let text = trimmed
        .strip_prefix("```json")
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    let summary: Summary = serde_json::from_str(text)
        .map_err(|_| "The result summary did not provide verifiable sourced claims.".to_string())?;
    if summary.coverage != "complete" || !summary.missing.is_empty() {
        return Err("The returned results do not establish every requested detail.".into());
    }
    if summary.claims.is_empty() || summary.claims.len() > 12 {
        return Err("The result summary must provide one to twelve sourced claims.".into());
    }
    let mut lines = Vec::new();
    for claim in summary.claims {
        let text = claim.text.trim();
        if text.is_empty()
            || text.chars().count() > 8000
            || claim.sources.is_empty()
            || claim.sources.len() > card_count
            || text.split('[').skip(1).any(|suffix| {
                suffix.split_once(']').is_some_and(|(number, _)| {
                    !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
                })
            })
        {
            return Err("Each result claim needs plain text and explicit source numbers.".into());
        }
        let mut observed = std::collections::BTreeSet::new();
        let mut citations = Vec::new();
        for source in claim.sources {
            let number = source
                .parse::<usize>()
                .map_err(|_| "A result claim contains an invalid source number.".to_string())?;
            if number == 0
                || number > card_count
                || number.to_string() != source
                || !observed.insert(number)
            {
                return Err("A result claim cites a source outside its returned results.".into());
            }
            citations.push(format!("[{number}]"));
        }
        lines.push(format!("{text} {}", citations.join(" ")));
    }
    Ok(lines.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary(claims: serde_json::Value) -> String {
        json!({"claims":claims,"coverage":"complete","missing":[]}).to_string()
    }

    #[test]
    fn summaries_render_only_model_selected_known_source_numbers() {
        let raw = summary(json!([
            {"text":"Seattle is overcast.","sources":["1"]},
            {"text":"Both cafes have street addresses.","sources":["2","3"]}
        ]));
        assert_eq!(
            render_card_summary(&raw, 3).unwrap(),
            "Seattle is overcast. [1]\n\nBoth cafes have street addresses. [2] [3]"
        );
        for sources in [
            json!([]),
            json!(["0"]),
            json!(["4"]),
            json!(["01"]),
            json!(["1", "1"]),
            json!(["https://example.com"]),
        ] {
            assert!(render_card_summary(
                &summary(json!([{"text":"A claim","sources":sources}])),
                3
            )
            .is_err());
        }
    }

    #[test]
    fn summaries_reject_uncited_prose_partial_coverage_and_embedded_citations() {
        for raw in [
            "A free-form answer with no source metadata.".to_string(),
            summary(json!([])),
            summary(json!([{"text":"","sources":["1"]}])),
            summary(json!([{"text":"An unsupported marker [99].","sources":["1"]}])),
            json!({"claims":[{"text":"One fact","sources":["1"]}],"coverage":"partial","missing":["Second requested fact"]}).to_string(),
            json!({"claims":[{"text":"One fact","sources":["1"]}],"coverage":"complete","missing":[],"answer":"Unverified extra prose"}).to_string(),
        ] {
            assert!(render_card_summary(&raw,1).is_err(),"{raw}");
        }
    }
}
