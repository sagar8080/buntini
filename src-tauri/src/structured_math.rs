//! Small, deterministic table operations. The model extracts source facts; this
//! module validates their shape and computes relationships without evaluating
//! expressions, reading files, making network requests or performing effects.
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const TOOLS: &[&str] = &[
    "split_costs",
    "scale_quantities",
    "compare_options",
    "sum_by_status",
];
const MAX_ROWS: usize = 24;
const MAX_PARTICIPANTS: usize = 16;
const MAX_ROW_BYTES: usize = 512;
const MAX_LABEL_BYTES: usize = 120;
const MAX_INPUT: f64 = 1e12;
const MAX_RESULT: f64 = 1e15;

fn object<'a>(
    args: &'a Value,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let fields = args
        .as_object()
        .ok_or("Math arguments must be an object.")?;
    if required.iter().any(|key| !fields.contains_key(*key))
        || fields
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err("Math arguments have missing or unknown fields.".into());
    }
    Ok(fields)
}

fn label(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_LABEL_BYTES
        || value
            .chars()
            .any(|c| c.is_control() || c == '|' || c == ',')
    {
        return Err(
            "Labels must contain 1–120 bytes without control characters, commas or pipes.".into(),
        );
    }
    Ok(value.into())
}

fn text<'a>(fields: &'a Map<String, Value>, name: &str) -> Result<&'a str, String> {
    fields
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be text."))
}

fn rows(fields: &Map<String, Value>, name: &str, max: usize) -> Result<Vec<String>, String> {
    let items = fields
        .get(name)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{name} must be a string array."))?;
    if items.is_empty() || items.len() > max {
        return Err(format!("{name} requires 1–{max} rows."));
    }
    items
        .iter()
        .map(|row| {
            let row = row.as_str().ok_or("Every row must be text.")?;
            if row.is_empty() || row.len() > MAX_ROW_BYTES || row.chars().any(char::is_control) {
                return Err("Each row must contain 1–512 bytes without control characters.".into());
            }
            Ok(row.to_owned())
        })
        .collect()
}

fn cells(row: &str) -> Result<[&str; 3], String> {
    let pieces: Vec<_> = row.split('|').map(str::trim).collect();
    if pieces.len() != 3 {
        return Err("Each row must contain exactly three pipe-separated cells.".into());
    }
    Ok([pieces[0], pieces[1], pieces[2]])
}

fn numeric(value: &str, negative: bool) -> Result<f64, String> {
    let value = value.trim().strip_prefix('$').unwrap_or(value.trim());
    // Decimal numeric cells only. No formulas, grouping, units or executable syntax.
    if value.is_empty() || value.len() > 48 || !value.is_ascii() {
        return Err("Numeric cells must be bounded decimal literals.".into());
    }
    let digits = value
        .strip_prefix('-')
        .filter(|_| negative)
        .unwrap_or(value);
    let mut dots = 0;
    if !digits.bytes().all(|c| {
        if c == b'.' {
            dots += 1;
            dots <= 1
        } else {
            c.is_ascii_digit()
        }
    }) || !digits.bytes().any(|c| c.is_ascii_digit())
    {
        return Err("Numeric cells must be decimal literals, without formulas or units.".into());
    }
    let result = value
        .parse::<f64>()
        .map_err(|_| "Invalid decimal numeric cell.")?;
    if !result.is_finite() || result.abs() > MAX_INPUT || (!negative && result < 0.0) {
        return Err("Numeric input exceeds the supported finite range.".into());
    }
    Ok(if result == 0.0 { 0.0 } else { result })
}

fn number(
    fields: &Map<String, Value>,
    name: &str,
    positive: bool,
    negative: bool,
) -> Result<f64, String> {
    let result = fields
        .get(name)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("{name} must be a number."))?;
    if !result.is_finite()
        || result.abs() > MAX_INPUT
        || (!negative && result < 0.0)
        || (positive && result <= 0.0)
    {
        return Err(format!("{name} is outside the supported finite range."));
    }
    Ok(result)
}

fn finite(value: f64) -> Result<f64, String> {
    if value.is_finite() && value.abs() <= MAX_RESULT {
        Ok(if value == 0.0 { 0.0 } else { value })
    } else {
        Err("Math result exceeds the supported finite range.".into())
    }
}

fn display(value: f64) -> String {
    if value != 0.0 && (value.abs() < 1e-6 || value.abs() >= 1e15) {
        let scientific = format!("{value:.14e}");
        let (mantissa, exponent) = scientific.split_once('e').unwrap();
        format!(
            "{}e{}",
            mantissa.trim_end_matches('0').trim_end_matches('.'),
            exponent
        )
    } else {
        format!("{value:.12}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}

fn unique(seen: &mut BTreeSet<String>, value: &str) -> Result<(), String> {
    if !seen.insert(value.to_lowercase()) {
        Err("Duplicate labels or participants are ambiguous; give distinct names.".into())
    } else {
        Ok(())
    }
}

fn split(args: &Value) -> Result<(Value, String), String> {
    let fields = object(args, &["participants", "charges"], &[])?;
    let participants = rows(fields, "participants", MAX_PARTICIPANTS)?
        .iter()
        .map(|value| label(value))
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = BTreeSet::new();
    for participant in &participants {
        unique(&mut seen, participant)?;
        if participant == "*" {
            return Err("A participant cannot be the all-participants marker.".into());
        }
    }
    let mut totals = vec![0.0; participants.len()];
    let mut common = 0.0;
    let mut charges = Vec::new();
    let mut names = BTreeSet::new();
    let mut arithmetic = Vec::new();
    for row in rows(fields, "charges", MAX_ROWS)? {
        let [name, amount, beneficiaries] = cells(&row)?;
        let name = label(name)?;
        unique(&mut names, &name)?;
        let amount = numeric(amount, false)?;
        let indices = if beneficiaries == "*" {
            (0..participants.len()).collect::<Vec<_>>()
        } else {
            let mut selected = BTreeSet::new();
            for person in beneficiaries.split(',') {
                let person = label(person)?;
                let index = participants
                    .iter()
                    .position(|value| value.to_lowercase() == person.to_lowercase())
                    .ok_or("A charge names an unknown participant.")?;
                if !selected.insert(index) {
                    return Err("A charge lists the same participant more than once.".into());
                }
            }
            selected.into_iter().collect()
        };
        if indices.is_empty() {
            return Err("Each charge requires at least one beneficiary.".into());
        }
        let share = finite(amount / indices.len() as f64)?;
        if indices.len() == participants.len() {
            common = finite(common + share)?;
        }
        for index in &indices {
            totals[*index] = finite(totals[*index] + share)?;
        }
        arithmetic.push(format!(
            "{name}: {} / {} = {} per beneficiary.",
            display(amount),
            indices.len(),
            display(share)
        ));
        charges.push(json!({"label":name,"amount":amount,"beneficiaries":indices.iter().map(|index|&participants[*index]).collect::<Vec<_>>(),"share":share}));
    }
    let result = participants
        .iter()
        .zip(&totals)
        .map(|(name, total)| json!({"participant":name,"total":total}))
        .collect::<Vec<_>>();
    let mut answer = format!(
        "Shared base per person: {}.\n{}",
        display(common),
        arithmetic.join("\n")
    );
    for (name, total) in participants.iter().zip(totals) {
        answer.push_str(&format!("\n{name} owes {}.", display(total)));
    }
    Ok((
        json!({"charges":charges,"sharedBase":common,"totals":result}),
        answer,
    ))
}

fn scale(args: &Value) -> Result<(Value, String), String> {
    let fields = object(args, &["from_servings", "to_servings", "items"], &[])?;
    let from = number(fields, "from_servings", true, false)?;
    let to = number(fields, "to_servings", true, false)?;
    if from < 1e-6 || to < 1e-6 || from > 1e6 || to > 1e6 {
        return Err("Serving counts must be between 0.000001 and 1000000.".into());
    }
    let ratio = finite(to / from)?;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    let mut answer = Vec::new();
    for row in rows(fields, "items", MAX_ROWS)? {
        let [name, quantity, unit] = cells(&row)?;
        let name = label(name)?;
        unique(&mut seen, &name)?;
        let unit = if unit.is_empty() {
            String::new()
        } else {
            label(unit)?
        };
        let quantity = numeric(quantity, false)?;
        let scaled = finite(quantity * ratio)?;
        answer.push(if unit.is_empty() {
            format!("- {} {name}", display(scaled))
        } else {
            format!("- {} {unit} {name}", display(scaled))
        });
        result
            .push(json!({"label":name,"originalQuantity":quantity,"quantity":scaled,"unit":unit}));
    }
    Ok((
        json!({"fromServings":from,"toServings":to,"factor":ratio,"items":result}),
        answer.join("\n"),
    ))
}

fn compare(args: &Value) -> Result<(Value, String), String> {
    let fields = object(args, &["minimum", "options"], &["attribute_label", "unit"])?;
    let minimum = number(fields, "minimum", false, true)?;
    let attribute = fields
        .get("attribute_label")
        .map(|_| label(text(fields, "attribute_label")?))
        .transpose()?
        .unwrap_or("attribute".into());
    let unit = fields
        .get("unit")
        .map(|_| label(text(fields, "unit")?))
        .transpose()?
        .unwrap_or_default();
    let mut seen = BTreeSet::new();
    let mut options = Vec::new();
    for row in rows(fields, "options", MAX_ROWS)? {
        let [name, price, value] = cells(&row)?;
        let name = label(name)?;
        unique(&mut seen, &name)?;
        options.push((name, numeric(price, false)?, numeric(value, true)?));
    }
    let cheapest = options
        .iter()
        .filter(|(_, _, value)| *value >= minimum)
        .map(|(_, price, _)| *price)
        .min_by(f64::total_cmp);
    let winners = options
        .iter()
        .filter(|(_, price, value)| *value >= minimum && Some(*price) == cheapest)
        .map(|(name, _, _)| name.clone())
        .collect::<Vec<_>>();
    let mut answer = vec![format!(
        "Minimum requirement: {}{} {}.",
        display(minimum),
        unit,
        attribute
    )];
    if let Some(price) = cheapest {
        answer.push(format!(
            "{} is the cheapest qualifying option at {}.",
            winners.join(" and "),
            display(price)
        ));
    } else {
        answer.push("No supplied option meets the minimum requirement.".into());
    }
    let mut result = Vec::new();
    for (name, price, value) in options {
        let qualifies = value >= minimum;
        let difference = if qualifies {
            cheapest.map(|best| price - best)
        } else {
            None
        };
        if !qualifies {
            answer.push(format!(
                "{name} is excluded: {}{} {} is below {}{}.",
                display(value),
                unit,
                attribute,
                display(minimum),
                unit
            ));
        } else if let Some(difference) = difference.filter(|difference| *difference > 0.0) {
            answer.push(format!(
                "{} is {} less than {name} ({}).",
                winners.join(" and "),
                display(difference),
                display(price)
            ));
        }
        result.push(json!({"label":name,"price":price,"attributeValue":value,"qualifies":qualifies,"differenceFromCheapest":difference}));
    }
    Ok((
        json!({"minimum":minimum,"attributeLabel":attribute,"unit":unit,"cheapestPrice":cheapest,"winners":winners,"options":result}),
        answer.join("\n"),
    ))
}

fn sum_status(args: &Value) -> Result<(Value, String), String> {
    let fields = object(args, &["include_status", "items"], &["periods_per_year"])?;
    let status = label(text(fields, "include_status")?)?;
    let periods = if fields.contains_key("periods_per_year") {
        number(fields, "periods_per_year", true, false)?
    } else {
        12.0
    };
    if periods > 10_000.0 {
        return Err("periods_per_year exceeds 10000.".into());
    }
    let mut seen = BTreeSet::new();
    let mut selected = Vec::new();
    let mut excluded = Vec::new();
    let mut result = Vec::new();
    let mut total = 0.0;
    for row in rows(fields, "items", MAX_ROWS)? {
        let [name, amount, row_status] = cells(&row)?;
        let name = label(name)?;
        unique(&mut seen, &name)?;
        let row_status = label(row_status)?;
        let amount = numeric(amount, false)?;
        let included = row_status.to_lowercase() == status.to_lowercase();
        if included {
            total = finite(total + amount)?;
            selected.push(name.clone());
        } else {
            excluded.push(name.clone());
        }
        result.push(json!({"label":name,"amount":amount,"status":row_status,"included":included}));
    }
    let yearly = finite(total * periods)?;
    let period = if periods == 12.0 {
        "month"
    } else if periods == 52.0 {
        "week"
    } else if periods == 4.0 {
        "quarter"
    } else if periods == 365.0 {
        "day"
    } else {
        "period"
    };
    let answer = format!("Selected items ({status}): {}.\nOther items kept: {}.\nSelected total per {period}: {}.\nSelected total per year: {} ({} × {}).", if selected.is_empty(){"none".into()}else{selected.join(", ")}, if excluded.is_empty(){"none".into()}else{excluded.join(", ")},display(total),display(yearly),display(total),display(periods));
    Ok((
        json!({"includeStatus":status,"periodsPerYear":periods,"items":result,"selected":selected,"excluded":excluded,"perPeriod":total,"perYear":yearly}),
        answer,
    ))
}

pub fn run(tool: &str, arguments: &Value) -> Result<Value, String> {
    let (results, answer) = match tool {
        "split_costs" => split(arguments),
        "scale_quantities" => scale(arguments),
        "compare_options" => compare(arguments),
        "sum_by_status" => sum_status(arguments),
        _ => Err("Unknown structured math tool.".into()),
    }?;
    Ok(
        json!({"kind":"structured_math_result","tool":tool,"inputs":arguments,"results":results,"answer":answer,"precision":"Finite decimal inputs, IEEE 754 arithmetic; display rounded to 12 decimal places. All amounts must use the same source units; no conversion is performed."}),
    )
}

pub fn validate(tool: &str, arguments: &Value) -> Result<(), String> {
    run(tool, arguments).map(|_| ())
}
pub fn verify(tool: &str, arguments: &Value, evidence: &Value) -> bool {
    run(tool, arguments).is_ok_and(|expected| expected == *evidence)
}
pub fn render(evidence: &Value) -> String {
    if evidence["kind"] == "structured_math_result" {
        evidence["answer"].as_str().unwrap_or_default().into()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_applies_each_cost_only_to_its_named_beneficiaries() {
        let args = json!({"participants":["Aria","Bo","Cy"],"charges":["Lodging|90|*","Food|27|*","Parking|14|Aria,Bo"]});
        let result = run("split_costs", &args).unwrap();
        assert_eq!(
            result["results"]["totals"],
            json!([{"participant":"Aria","total":46.0},{"participant":"Bo","total":46.0},{"participant":"Cy","total":39.0}])
        );
        assert!(render(&result).contains("Parking: 14 / 2 = 7"));
        assert!(verify("split_costs", &args, &result));
    }
    #[test]
    fn scaling_uses_one_ratio_and_preserves_units_and_labels() {
        let result=run("scale_quantities",&json!({"from_servings":3,"to_servings":9,"items":["Rice|75|g","Beans|120|g","Limes|2|"]})).unwrap();
        assert_eq!(result["results"]["factor"], 3.0);
        assert_eq!(result["results"]["items"][0]["quantity"], 225.0);
        assert_eq!(render(&result), "- 225 g Rice\n- 360 g Beans\n- 6 Limes");
    }
    #[test]
    fn comparison_filters_before_price_selection_and_keeps_ties_and_no_matches_honest() {
        let args = json!({"minimum":24,"attribute_label":"capacity","unit":"L","options":["Compact|40|12","Daypack|65|24","Trail|80|30","Twin|65|26"]});
        let result = run("compare_options", &args).unwrap();
        assert_eq!(result["results"]["winners"], json!(["Daypack", "Twin"]));
        assert_eq!(
            result["results"]["options"][2]["differenceFromCheapest"],
            15.0
        );
        assert!(render(&result).contains("Compact is excluded"));
        let no_match = run(
            "compare_options",
            &json!({"minimum":50,"options":["Small|20|12"]}),
        )
        .unwrap();
        assert_eq!(no_match["results"]["winners"], json!([]));
        assert!(render(&no_match).contains("No supplied option"));
    }
    #[test]
    fn status_sum_selects_exact_status_and_annualizes_once() {
        let result=run("sum_by_status",&json!({"include_status":"inactive","periods_per_year":52,"items":["Library|4|active","Delivery|6|inactive","Games|9|inactive"]})).unwrap();
        assert_eq!(result["results"]["selected"], json!(["Delivery", "Games"]));
        assert_eq!(result["results"]["perPeriod"], 15.0);
        assert_eq!(result["results"]["perYear"], 780.0);
        assert!(render(&result).contains("Other items kept: Library"));
    }
    #[test]
    fn forged_results_inputs_and_answer_text_never_verify() {
        let args = json!({"from_servings":4,"to_servings":10,"items":["Flour|80|g"]});
        for field in ["results", "inputs", "answer", "tool"] {
            let mut value = run("scale_quantities", &args).unwrap();
            value[field] = json!("forged");
            assert!(!verify("scale_quantities", &args, &value));
        }
        assert!(render(&json!({"kind":"other","answer":"made up"})).is_empty());
    }
    #[test]
    fn malformed_rows_duplicates_unknown_people_and_non_numeric_cells_fail_closed() {
        for args in [
            json!({"participants":["Aria","aria"],"charges":["Bill|12|*"]}),
            json!({"participants":["Aria"],"charges":["Bill|12|Missing"]}),
            json!({"participants":["Aria"],"charges":["Bill|12|Aria,Aria"]}),
            json!({"participants":["Aria"],"charges":["Bill|12|*"] ,"command":"bad"}),
            json!({"participants":["Aria"],"charges":["Bill|$(id)|*"]}),
            json!({"participants":["Aria"],"charges":["Bill|12|*|extra"]}),
        ] {
            assert!(run("split_costs", &args).is_err(), "{args}");
        }
        for value in [
            "NaN",
            "inf",
            "1e999",
            "3+4",
            "1/2",
            "-3",
            "3kg",
            "3,000",
            "$$3",
            "0.000000000000000000000000000000000000000000000000001",
        ] {
            assert!(numeric(value, false).is_err(), "{value}");
        }
        assert!(numeric("$43.50", false).is_ok());
        assert!(run(
            "scale_quantities",
            &json!({"from_servings":0,"to_servings":3,"items":["Rice|1|g"]})
        )
        .is_err());
        assert!(run(
            "sum_by_status",
            &json!({"include_status":"x","items":["One|1|x","one|2|x"]})
        )
        .is_err());
    }
    #[test]
    fn finite_values_and_resource_limits_are_checked_before_any_receipt() {
        assert!(run(
            "scale_quantities",
            &json!({"from_servings":0.000001,"to_servings":1e12,"items":["Flour|1e12|g"]})
        )
        .is_err());
        assert!(run(
            "scale_quantities",
            &json!({"from_servings":1,"to_servings":2,"items":vec!["x|1|g";25]})
        )
        .is_err());
        assert!(run(
            "split_costs",
            &json!({"participants":vec!["x";17],"charges":["x|1|*"]})
        )
        .is_err());
        assert!(run(
            "compare_options",
            &json!({"minimum":1,"options":[format!("{}|1|2","x".repeat(513))]})
        )
        .is_err());
        assert!(run(
            "sum_by_status",
            &json!({"include_status":"x","items":["x|1|x"],"periods_per_year":10001})
        )
        .is_err());
        assert!(run("unknown", &json!({})).is_err());
    }
}
