use serde_json::Value as Json;

use crate::data_types::DataType;
use crate::value::Value;

pub(super) fn check(results: &str) {
    let mut failures = Vec::new();
    let mut total = 0;
    for line in results.lines() {
        let case: Json = serde_json::from_str(line).expect("result lines are JSON");
        total += 1;
        if let Err(e) = check_case(&case) {
            failures.push(format!("{} / {}: {e}", case["type"], case["case"]));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {total} cases failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn check_case(case: &Json) -> Result<(), String> {
    let ty_name = case["type"].as_str().expect("type is a string");
    let ty = DataType::parse(ty_name);
    if let DataType::Other(_) = ty {
        return Err(format!("`{ty_name}` doesn't parse to a supported type"));
    }
    let accepted = case["accepted"].as_bool().expect("accepted is a bool");

    let event = match case["event"].as_str() {
        None => None,
        Some(raw) => match crate::parse_event(raw) {
            Some(json) => Some(json),
            None if accepted => {
                return Err(format!(
                    "the Sink accepted `{raw}` but parse_event rejects it"
                ));
            }
            None => return Ok(()),
        },
    };

    let expected = if accepted {
        let history = &case["history"];
        match Value::from_history(history, &ty) {
            Ok(v) => Some(v),
            Err(e) => return Err(e.to_string()),
        }
    } else {
        None
    };
    let actual = Value::from_event(event.as_ref(), &ty);
    if same(&actual, &expected) {
        Ok(())
    } else {
        let event = case["event"].as_str().unwrap_or("<absent>");
        Err(format!(
            "from_event({event}) = {actual:?}, expected {expected:?}{}",
            case["error"]
                .as_str()
                .map(|e| format!(" (the Sink rejected it: {e})"))
                .unwrap_or_default()
        ))
    }
}

fn same(a: &Option<Value>, b: &Option<Value>) -> bool {
    match (a, b) {
        (Some(Value::Float(x)), Some(Value::Float(y))) => {
            x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan())
        }
        _ => a == b,
    }
}
