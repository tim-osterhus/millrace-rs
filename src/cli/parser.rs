use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyValue {
    Allow,
    NonEmpty,
    NonBlank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptionSpec {
    name: &'static str,
    takes_value: bool,
    repeatable: bool,
    empty_value: EmptyValue,
}

impl OptionSpec {
    pub const fn value(name: &'static str, empty_value: EmptyValue) -> Self {
        Self {
            name,
            takes_value: true,
            repeatable: false,
            empty_value,
        }
    }

    pub const fn repeatable_value(name: &'static str, empty_value: EmptyValue) -> Self {
        Self {
            name,
            takes_value: true,
            repeatable: true,
            empty_value,
        }
    }

    pub const fn flag(name: &'static str) -> Self {
        Self {
            name,
            takes_value: false,
            repeatable: false,
            empty_value: EmptyValue::Allow,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedArgs {
    values: BTreeMap<String, Vec<Option<String>>>,
    pub positionals: Vec<String>,
}

impl ParsedArgs {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .and_then(|values| values.last())
            .and_then(|value| value.as_deref())
    }

    pub fn values(&self, name: &str) -> Vec<&str> {
        self.values
            .get(name)
            .map(|values| values.iter().filter_map(|value| value.as_deref()).collect())
            .unwrap_or_default()
    }

    pub fn has(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }
}

pub fn parse_args(
    args: impl IntoIterator<Item = String>,
    specs: &[OptionSpec],
) -> Result<ParsedArgs, String> {
    let mut args = args.into_iter();
    let mut parsed = ParsedArgs::default();

    while let Some(arg) = args.next() {
        if let Some((name, inline_value)) = split_long_option_with_value(&arg) {
            let spec = find_spec(specs, name).ok_or_else(|| format!("unknown option `{name}`"))?;
            if !spec.takes_value {
                return Err(format!("option `{name}` does not take a value"));
            }
            insert_option_value(&mut parsed, spec, inline_value.to_owned())?;
            continue;
        }

        if arg.starts_with("--") {
            let spec = find_spec(specs, &arg).ok_or_else(|| format!("unknown option `{arg}`"))?;
            if spec.takes_value {
                let Some(value) = args.next() else {
                    return Err(format!("missing value for `{}`", spec.name));
                };
                if value.starts_with('-') {
                    return Err(format!("missing value for `{}`", spec.name));
                }
                insert_option_value(&mut parsed, spec, value)?;
            } else {
                insert_flag(&mut parsed, spec)?;
            }
            continue;
        }

        if arg.starts_with('-') {
            return Err(format!("unknown option `{arg}`"));
        }

        parsed.positionals.push(arg);
    }

    Ok(parsed)
}

fn split_long_option_with_value(arg: &str) -> Option<(&str, &str)> {
    let (name, value) = arg.split_once('=')?;
    if name.starts_with("--") {
        Some((name, value))
    } else {
        None
    }
}

fn find_spec<'a>(specs: &'a [OptionSpec], name: &str) -> Option<&'a OptionSpec> {
    specs.iter().find(|spec| spec.name == name)
}

fn insert_option_value(
    parsed: &mut ParsedArgs,
    spec: &OptionSpec,
    value: String,
) -> Result<(), String> {
    reject_duplicate(parsed, spec)?;
    validate_value(spec, &value)?;
    parsed
        .values
        .entry(spec.name.to_owned())
        .or_default()
        .push(Some(value));
    Ok(())
}

fn insert_flag(parsed: &mut ParsedArgs, spec: &OptionSpec) -> Result<(), String> {
    reject_duplicate(parsed, spec)?;
    parsed
        .values
        .entry(spec.name.to_owned())
        .or_default()
        .push(None);
    Ok(())
}

fn reject_duplicate(parsed: &ParsedArgs, spec: &OptionSpec) -> Result<(), String> {
    if !spec.repeatable && parsed.values.contains_key(spec.name) {
        return Err(format!("duplicate `{}` option", spec.name));
    }
    Ok(())
}

fn validate_value(spec: &OptionSpec, value: &str) -> Result<(), String> {
    match spec.empty_value {
        EmptyValue::Allow => Ok(()),
        EmptyValue::NonEmpty if value.is_empty() => {
            Err(format!("`{}` value must not be empty", spec.name))
        }
        EmptyValue::NonBlank if value.trim().is_empty() => {
            Err(format!("`{}` value must not be empty", spec.name))
        }
        _ => Ok(()),
    }
}
