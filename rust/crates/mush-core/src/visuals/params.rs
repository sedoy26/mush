//! Parameter system for tunable visual effects.

/// Specification for a tunable parameter.
#[derive(Clone, Debug)]
pub struct ParamSpec {
    /// Internal identifier used in set_param/get_param
    pub key: &'static str,
    /// Human-readable label for UI
    pub label: &'static str,
    /// Type and constraints
    pub kind: ParamKind,
    /// Initial/default value
    pub default: ParamValue,
}

/// Parameter type with constraints.
#[derive(Clone, Debug)]
pub enum ParamKind {
    Float { min: f32, max: f32, step: f32 },
    Int { min: i32, max: i32 },
    Bool,
    Enum { variants: &'static [&'static str] },
}

/// Parameter value.
#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    /// Index into the enum's variants array
    Enum(usize),
}

impl ParamValue {
    /// Get as float, or None if wrong type.
    pub fn as_float(&self) -> Option<f32> {
        match self {
            ParamValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as int, or None if wrong type.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            ParamValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as bool, or None if wrong type.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// Get as enum index, or None if wrong type.
    pub fn as_enum(&self) -> Option<usize> {
        match self {
            ParamValue::Enum(v) => Some(*v),
            _ => None,
        }
    }
}

/// Error when setting a parameter.
#[derive(Clone, Debug)]
pub struct ParamError {
    pub message: String,
}

impl ParamError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn unknown_key(key: &str) -> Self {
        Self::new(format!("unknown parameter: {}", key))
    }

    pub fn type_mismatch(key: &str, expected: &str) -> Self {
        Self::new(format!("parameter '{}' expects {}", key, expected))
    }

    pub fn out_of_range(key: &str, value: f32, min: f32, max: f32) -> Self {
        Self::new(format!(
            "parameter '{}' value {} out of range [{}, {}]",
            key, value, min, max
        ))
    }
}

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ParamError {}

/// Helper trait for effects to find and update parameters.
#[allow(dead_code)]
pub trait ParamHelpers {
    fn find_param(&self, key: &str) -> Option<&ParamSpec>;
    
    fn validate_float(&self, key: &str, value: f32) -> Result<f32, ParamError> {
        match self.find_param(key) {
            Some(spec) => match spec.kind {
                ParamKind::Float { min, max, .. } => {
                    if value < min || value > max {
                        Err(ParamError::out_of_range(key, value, min, max))
                    } else {
                        Ok(value)
                    }
                }
                _ => Err(ParamError::type_mismatch(key, "float")),
            },
            None => Err(ParamError::unknown_key(key)),
        }
    }
    
    fn validate_int(&self, key: &str, value: i32) -> Result<i32, ParamError> {
        match self.find_param(key) {
            Some(spec) => match spec.kind {
                ParamKind::Int { min, max } => {
                    let clamped = value.clamp(min, max);
                    Ok(clamped)
                }
                _ => Err(ParamError::type_mismatch(key, "int")),
            },
            None => Err(ParamError::unknown_key(key)),
        }
    }
    
    fn validate_enum(&self, key: &str, value: usize) -> Result<usize, ParamError> {
        match self.find_param(key) {
            Some(spec) => match spec.kind {
                ParamKind::Enum { variants } => {
                    if value >= variants.len() {
                        Err(ParamError::new(format!(
                            "enum index {} out of range for '{}'",
                            value, key
                        )))
                    } else {
                        Ok(value)
                    }
                }
                _ => Err(ParamError::type_mismatch(key, "enum")),
            },
            None => Err(ParamError::unknown_key(key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_param_value() {
        let f = ParamValue::Float(0.5);
        assert_eq!(f.as_float(), Some(0.5));
        assert_eq!(f.as_int(), None);
        
        let i = ParamValue::Int(42);
        assert_eq!(i.as_int(), Some(42));
        assert_eq!(i.as_float(), None);
        
        let b = ParamValue::Bool(true);
        assert_eq!(b.as_bool(), Some(true));
        
        let e = ParamValue::Enum(2);
        assert_eq!(e.as_enum(), Some(2));
    }
}
