use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Formatter};
use std::path::PathBuf;
use std::rc::Rc;

use alacritty_config::SerdeReplace;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::Osc52;
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use serde::de::{Error as SerdeError, MapAccess, Visitor};
use serde::{self, Deserialize, Deserializer};
use unicode_width::UnicodeWidthChar;
use winit::keyboard::{Key, ModifiersState};

use alacritty_config_derive::{ConfigDeserialize, SerdeReplace};

use crate::config::bindings::{
    self, Binding, BindingKey, KeyBinding, KeyLocation, ModeWrapper, ModsWrapper,
};
use crate::config::color::Colors;
use crate::config::cursor::Cursor;
use crate::config::debug::Debug;
use crate::config::font::Font;
use crate::config::mouse::Mouse;
use crate::config::selection::Selection;
use crate::config::window::WindowConfig;

/// Regex used for the default URL hint.
#[rustfmt::skip]
const URL_REGEX: &str = "(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file:|git://|ssh:|ftp://)\
                         [^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`]+";

#[derive(ConfigDeserialize, Clone, Debug, PartialEq)]
pub struct UiConfig {
    pub colors: Colors,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { colors: Default::default() }
    }
}

/// Keyboard configuration.
#[derive(ConfigDeserialize, Default, Clone, Debug, PartialEq)]
struct Keyboard {
    /// Keybindings.
    bindings: KeyBindings,
}

#[derive(SerdeReplace, Clone, Debug, PartialEq, Eq)]
struct KeyBindings(Vec<KeyBinding>);

impl Default for KeyBindings {
    fn default() -> Self {
        Self(bindings::default_key_bindings())
    }
}

impl<'de> Deserialize<'de> for KeyBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(deserialize_bindings(deserializer, Self::default().0)?))
    }
}

pub fn deserialize_bindings<'a, D, T>(
    deserializer: D,
    mut default: Vec<Binding<T>>,
) -> Result<Vec<Binding<T>>, D::Error>
where
    D: Deserializer<'a>,
    T: Clone + Eq,
    Binding<T>: Deserialize<'a>,
{
    let values = Vec::<toml::Value>::deserialize(deserializer)?;

    // Skip all invalid values.
    let mut bindings = Vec::with_capacity(values.len());
    for value in values {
        match Binding::<T>::deserialize(value) {
            Ok(binding) => bindings.push(binding),
            Err(_) => {},
        }
    }

    // Remove matching default bindings.
    for binding in bindings.iter() {
        default.retain(|b| !b.triggers_match(binding));
    }

    bindings.extend(default);

    Ok(bindings)
}

/// A delta for a point in a 2 dimensional plane.
#[derive(ConfigDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delta<T: Default> {
    /// Horizontal change.
    pub x: T,
    /// Vertical change.
    pub y: T,
}

/// Regex terminal hints.
#[derive(ConfigDeserialize, Clone, Debug, PartialEq, Eq)]
pub struct Hints {
    /// Characters for the hint labels.
    alphabet: HintsAlphabet,

    /// All configured terminal hints.
    pub enabled: Vec<Hint>,
}

impl Default for Hints {
    fn default() -> Self {
        // Add URL hint by default when no other hint is present.
        let pattern = LazyRegexVariant::Pattern(String::from(URL_REGEX));
        let regex = LazyRegex(Rc::new(RefCell::new(pattern)));
        let content = HintContent::new(Some(regex), true);

        #[cfg(not(any(target_os = "macos", windows)))]
        let action = HintAction::Command(Program::Just(String::from("xdg-open")));
        #[cfg(target_os = "macos")]
        let action = HintAction::Command(Program::Just(String::from("open")));
        #[cfg(windows)]
        let action = HintAction::Command(Program::WithArgs {
            program: String::from("cmd"),
            args: vec!["/c".to_string(), "start".to_string(), "".to_string()],
        });

        Self {
            enabled: vec![Hint {
                content,
                action,
                persist: false,
                post_processing: true,
                mouse: Some(HintMouse { enabled: true, mods: Default::default() }),
                binding: Some(HintBinding {
                    key: BindingKey::Keycode {
                        key: Key::Character("u".into()),
                        location: KeyLocation::Standard,
                    },
                    mods: ModsWrapper(ModifiersState::SHIFT | ModifiersState::CONTROL),
                    mode: Default::default(),
                }),
            }],
            alphabet: Default::default(),
        }
    }
}

#[derive(SerdeReplace, Clone, Debug, PartialEq, Eq)]
struct HintsAlphabet(String);

impl Default for HintsAlphabet {
    fn default() -> Self {
        Self(String::from("jfkdls;ahgurieowpq"))
    }
}

impl<'de> Deserialize<'de> for HintsAlphabet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        let mut character_count = 0;
        for character in value.chars() {
            if character.width() != Some(1) {
                return Err(D::Error::custom("characters must be of width 1"));
            }
            character_count += 1;
        }

        if character_count < 2 {
            return Err(D::Error::custom("must include at last 2 characters"));
        }

        Ok(Self(value))
    }
}

/// Built-in actions for hint mode.
#[derive(ConfigDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum HintInternalAction {
    /// Copy the text to the clipboard.
    Copy,
    /// Write the text to the PTY/search.
    Paste,
    /// Select the text matching the hint.
    Select,
    /// Move the vi mode cursor to the beginning of the hint.
    MoveViModeCursor,
}

/// Actions for hint bindings.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum HintAction {
    /// Built-in hint action.
    #[serde(rename = "action")]
    Action(HintInternalAction),

    /// Command the text will be piped to.
    #[serde(rename = "command")]
    Command(Program),
}

/// Hint configuration.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Regex for finding matches.
    #[serde(flatten)]
    pub content: HintContent,

    /// Action executed when this hint is triggered.
    #[serde(flatten)]
    pub action: HintAction,

    /// Hint text post processing.
    #[serde(default)]
    pub post_processing: bool,

    /// Persist hints after selection.
    #[serde(default)]
    pub persist: bool,

    /// Hint mouse highlighting.
    pub mouse: Option<HintMouse>,

    /// Binding required to search for this hint.
    binding: Option<HintBinding>,
}

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct HintContent {
    /// Regex for finding matches.
    pub regex: Option<LazyRegex>,

    /// Escape sequence hyperlinks.
    pub hyperlinks: bool,
}

impl HintContent {
    pub fn new(regex: Option<LazyRegex>, hyperlinks: bool) -> Self {
        Self { regex, hyperlinks }
    }
}

impl<'de> Deserialize<'de> for HintContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct HintContentVisitor;
        impl<'a> Visitor<'a> for HintContentVisitor {
            type Value = HintContent;

            fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("a mapping")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'a>,
            {
                let mut content = Self::Value::default();

                while let Some((key, value)) = map.next_entry::<String, toml::Value>()? {
                    match key.as_str() {
                        "regex" => match Option::<LazyRegex>::deserialize(value) {
                            Ok(regex) => content.regex = regex,
                            Err(_) => {},
                        },
                        "hyperlinks" => match bool::deserialize(value) {
                            Ok(hyperlink) => content.hyperlinks = hyperlink,
                            Err(_) => {},
                        },
                        "command" | "action" => (),
                        _ => (),
                    }
                }

                // Require at least one of hyperlinks or regex trigger hint matches.
                if content.regex.is_none() && !content.hyperlinks {
                    return Err(M::Error::custom(
                        "Config error: At least on of the hint's regex or hint's hyperlinks must \
                         be set",
                    ));
                }

                Ok(content)
            }
        }

        deserializer.deserialize_any(HintContentVisitor)
    }
}

/// Binding for triggering a keyboard hint.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HintBinding {
    pub key: BindingKey,
    #[serde(default)]
    pub mods: ModsWrapper,
    #[serde(default)]
    pub mode: ModeWrapper,
}

/// Hint mouse highlighting.
#[derive(ConfigDeserialize, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub struct HintMouse {
    /// Hint mouse highlighting availability.
    pub enabled: bool,

    /// Required mouse modifiers for hint highlighting.
    pub mods: ModsWrapper,
}

/// Lazy regex with interior mutability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LazyRegex(Rc<RefCell<LazyRegexVariant>>);

impl<'de> Deserialize<'de> for LazyRegex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let regex = LazyRegexVariant::Pattern(String::deserialize(deserializer)?);
        Ok(Self(Rc::new(RefCell::new(regex))))
    }
}

/// Regex which is compiled on demand, to avoid expensive computations at startup.
#[derive(Clone, Debug)]
pub enum LazyRegexVariant {
    Pattern(String),
}

impl PartialEq for LazyRegexVariant {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Pattern(regex), Self::Pattern(other_regex)) => regex == other_regex,
        }
    }
}
impl Eq for LazyRegexVariant {}

/// Wrapper around f32 that represents a percentage value between 0.0 and 1.0.
#[derive(SerdeReplace, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Percentage(f32);

impl Default for Percentage {
    fn default() -> Self {
        Percentage(1.0)
    }
}

impl Percentage {
    pub fn new(value: f32) -> Self {
        Percentage(value.clamp(0., 1.))
    }

    pub fn as_f32(self) -> f32 {
        self.0
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged, deny_unknown_fields)]
pub enum Program {
    Just(String),
    WithArgs {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

impl Program {
    pub fn program(&self) -> &str {
        match self {
            Program::Just(program) => program,
            Program::WithArgs { program, .. } => program,
        }
    }

    pub fn args(&self) -> &[String] {
        match self {
            Program::Just(_) => &[],
            Program::WithArgs { args, .. } => args,
        }
    }
}

impl From<Program> for Shell {
    fn from(value: Program) -> Self {
        match value {
            Program::Just(program) => Shell::new(program, Vec::new()),
            Program::WithArgs { program, args } => Shell::new(program, args),
        }
    }
}

impl SerdeReplace for Program {
    fn replace(&mut self, value: toml::Value) -> Result<(), Box<dyn Error>> {
        *self = Self::deserialize(value)?;

        Ok(())
    }
}

pub(crate) struct StringVisitor;
impl<'de> serde::de::Visitor<'de> for StringVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a string")
    }

    fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(s.to_lowercase())
    }
}
