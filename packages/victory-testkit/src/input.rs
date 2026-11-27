//! Deterministic input compiled to the documented `VICTORY_KEYS` convention.
//!
//! The runtime reads `VICTORY_KEYS=<seconds>:<key>,...` and holds each key for
//! 0.2 seconds after that offset, with or without focus. Key names are a
//! letter, a digit, or `Enter`, `Esc`, `Tab`, `Back`, `Up`, `Down`, `Left` and
//! `Right`. This module accepts the same names, orders presses by offset and
//! virtual key, and emits one canonical environment value.
//!
//! The documented extension is that scenario steps give offsets as integer
//! milliseconds. They are written back as decimal seconds with three digits,
//! so one timeline always produces the same environment value. Frame anchored
//! input remains open because the runtime currently anchors to a clock.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Hold time the runtime applies to every scripted key, in milliseconds.
pub const HOLD_MS: u64 = 200;

/// A key press at an offset from the moment input starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyPress {
    /// Milliseconds after input starts.
    pub at_ms: u64,
    /// A documented key name or a single ASCII letter or digit.
    pub key: String,
}

impl KeyPress {
    /// Maps a documented key name to the virtual key the runtime uses.
    pub fn virtual_key(&self) -> Result<u8, InputError> {
        virtual_key(&self.key)
    }
}

/// A compiled script in canonical order, with its environment value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputScript {
    presses: Vec<KeyPress>,
    env_value: String,
}

impl InputScript {
    pub fn presses(&self) -> &[KeyPress] {
        &self.presses
    }

    /// The value to assign to `VICTORY_KEYS`, empty when there are no presses.
    pub fn env_value(&self) -> &str {
        &self.env_value
    }

    /// The environment entries this script contributes.
    pub fn env(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        if !self.env_value.is_empty() {
            env.insert("VICTORY_KEYS".into(), self.env_value.clone());
        }
        env
    }
}

/// Why an input script could not be compiled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    #[error("unknown key name {0:?}")]
    UnknownKey(String),
    #[error("duplicate press at offset {0}")]
    DuplicatePress(u64),
    #[error("invalid key offset {0:?}")]
    InvalidOffset(String),
}

/// Maps a documented key name to a Windows virtual key code.
pub fn virtual_key(key: &str) -> Result<u8, InputError> {
    let named = match key {
        "Enter" => 0x0D,
        "Esc" => 0x1B,
        "Tab" => 0x09,
        "Back" => 0x08,
        "Up" => 0x26,
        "Down" => 0x28,
        "Left" => 0x25,
        "Right" => 0x27,
        _ => return single_byte_key(key),
    };
    Ok(named)
}

fn single_byte_key(key: &str) -> Result<u8, InputError> {
    let bytes = key.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
        Ok(bytes[0].to_ascii_uppercase())
    } else {
        Err(InputError::UnknownKey(key.into()))
    }
}

/// The canonical name for a virtual key produced by [`virtual_key`].
pub fn canonical_name(vk: u8) -> Option<String> {
    let named = match vk {
        0x0D => "Enter",
        0x1B => "Esc",
        0x09 => "Tab",
        0x08 => "Back",
        0x26 => "Up",
        0x28 => "Down",
        0x25 => "Left",
        0x27 => "Right",
        _ => return None,
    };
    Some(named.into())
}

/// Compiles presses into a stable order and a canonical `VICTORY_KEYS` value.
pub fn compile(presses: &[KeyPress]) -> Result<InputScript, InputError> {
    let mut ordered: BTreeMap<(u64, u8), KeyPress> = BTreeMap::new();
    for press in presses {
        let vk = press.virtual_key()?;
        if ordered.insert((press.at_ms, vk), press.clone()).is_some() {
            return Err(InputError::DuplicatePress(press.at_ms));
        }
    }
    let entries: Vec<String> = ordered
        .keys()
        .map(|&(at_ms, vk)| {
            let name = canonical_name(vk)
                .unwrap_or_else(|| String::from_utf8(vec![vk]).unwrap_or_default());
            format!("{:.3}:{}", at_ms as f64 / 1000.0, name)
        })
        .collect();
    Ok(InputScript {
        presses: ordered.into_values().collect(),
        env_value: entries.join(","),
    })
}

/// Parses a `VICTORY_KEYS` value back into canonical presses.
pub fn parse_env(value: &str) -> Result<Vec<KeyPress>, InputError> {
    let mut presses = Vec::new();
    for entry in value.split(',').filter(|entry| !entry.is_empty()) {
        let Some((at, key)) = entry.split_once(':') else {
            return Err(InputError::InvalidOffset(entry.into()));
        };
        let Ok(seconds) = at.trim().parse::<f64>() else {
            return Err(InputError::InvalidOffset(at.into()));
        };
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(InputError::InvalidOffset(at.into()));
        }
        let vk = virtual_key(key.trim())?;
        let name =
            canonical_name(vk).unwrap_or_else(|| String::from_utf8(vec![vk]).unwrap_or_default());
        presses.push(KeyPress {
            at_ms: (seconds * 1000.0).round() as u64,
            key: name,
        });
    }
    Ok(presses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(at_ms: u64, key: &str) -> KeyPress {
        KeyPress {
            at_ms,
            key: key.into(),
        }
    }

    #[test]
    fn compiles_to_canonical_victory_keys() {
        let script = compile(&[press(3250, "Esc"), press(2000, "e"), press(500, "Enter")]).unwrap();
        assert_eq!(script.env_value(), "0.500:Enter,2.000:E,3.250:Esc");
        assert_eq!(
            script
                .presses()
                .iter()
                .map(KeyPress::virtual_key)
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            vec![0x0D, 0x45, 0x1B]
        );
        assert_eq!(
            script.env()["VICTORY_KEYS"],
            "0.500:Enter,2.000:E,3.250:Esc"
        );
    }

    #[test]
    fn compilation_is_order_independent() {
        let a = compile(&[press(100, "Tab"), press(900, "Right")]).unwrap();
        let b = compile(&[press(900, "Right"), press(100, "Tab")]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn rejects_unknown_keys() {
        let err = compile(&[press(100, "Space")]).unwrap_err();
        assert!(matches!(err, InputError::UnknownKey(_)), "{err}");
    }

    #[test]
    fn rejects_duplicate_presses() {
        let err = compile(&[press(100, "e"), press(100, "E")]).unwrap_err();
        assert!(matches!(err, InputError::DuplicatePress(100)), "{err}");
    }

    #[test]
    fn parses_its_own_output() {
        let script = compile(&[press(500, "Enter"), press(2000, "E")]).unwrap();
        let parsed = parse_env(script.env_value()).unwrap();
        assert_eq!(parsed, script.presses());
        assert!(compile(&[]).unwrap().env().is_empty());
    }
}
