//! The values a column takes from a list: an enum's, or true and false for
//! a boolean, written as each server reads them.

/// One value a cell can take from a list, and how the list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub label: String,
}

fn pairs(list: &[(&str, &str)]) -> Vec<Choice> {
    list.iter().map(|(value, label)| Choice { value: (*value).into(), label: (*label).into() }).collect()
}

/// The list for a column of `data_type` with enum `options` (from the table
/// structure); `None` for columns that take any text. SQL Server's `bit` and
/// MySQL's `tinyint(1)` hold a boolean as 1 or 0.
pub fn choices_of(data_type: &str, options: &[String]) -> Option<Vec<Choice>> {
    if !options.is_empty() {
        return Some(options.iter().map(|option| Choice { value: option.clone(), label: option.clone() }).collect());
    }
    match data_type.trim().to_lowercase().as_str() {
        "boolean" | "bool" => Some(pairs(&[("true", "true"), ("false", "false")])),
        "bit" | "tinyint(1)" => Some(pairs(&[("1", "true"), ("0", "false")])),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(choices: Option<Vec<Choice>>) -> Option<Vec<String>> {
        choices.map(|choices| choices.into_iter().map(|choice| choice.value).collect())
    }

    #[test]
    fn offers_an_enum_s_values_in_order() {
        let options = ["new".to_owned(), "it's done".to_owned()];
        assert_eq!(choices_of("dbforge.state", &options), Some(pairs(&[("new", "new"), ("it's done", "it's done")])));
        assert_eq!(choices_of("enum('a','b')", &[]), None);
    }

    #[test]
    fn offers_true_and_false_for_booleans_as_each_server_writes_them() {
        assert_eq!(values(choices_of("boolean", &[])), Some(vec!["true".into(), "false".into()]));
        assert_eq!(values(choices_of("bool", &[])), Some(vec!["true".into(), "false".into()]));
        assert_eq!(choices_of("bit", &[]), Some(pairs(&[("1", "true"), ("0", "false")])));
        assert_eq!(values(choices_of("tinyint(1)", &[])), Some(vec!["1".into(), "0".into()]));
        assert_eq!(choices_of("tinyint(4)", &[]), None);
        assert_eq!(choices_of("text", &[]), None);
    }
}
