//! Schema domains and visual presentation for intents: the system category an intent belongs to, and the title,
//! subtitle and symbol an entity or a result shows.
//!
//! Rules of this model:
//! - A domain is one of a closed set of categories ([`SchemaDomain::ALL`]). Its text name is lowercase and
//!   [`SchemaDomain::parse`] reads it back.
//! - A title is required. It is trimmed, holds 1 to 80 characters and has no control characters. A subtitle is optional
//!   and follows the same rules at up to 120 characters.
//! - A symbol names a glyph in the system symbol set: 1 to 64 lowercase ASCII letters, digits and dots, with no leading,
//!   trailing or doubled dot.
//!
//! This is the pure model. No schema registry or intent picker reads it yet, and no hardware entry point uses it.

/// Longest title, in characters after trimming.
pub const MAX_TITLE: usize = 80;
/// Longest subtitle, in characters after trimming.
pub const MAX_SUBTITLE: usize = 120;
/// Longest symbol name, in bytes.
pub const MAX_SYMBOL: usize = 64;

/// The system categories an intent can declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SchemaDomain {
    Browser,
    Calendar,
    Camera,
    Contacts,
    Documents,
    Files,
    Mail,
    Maps,
    Media,
    Messages,
    Notes,
    Photos,
    Reminders,
}

impl SchemaDomain {
    pub const ALL: [SchemaDomain; 13] = [
        SchemaDomain::Browser,
        SchemaDomain::Calendar,
        SchemaDomain::Camera,
        SchemaDomain::Contacts,
        SchemaDomain::Documents,
        SchemaDomain::Files,
        SchemaDomain::Mail,
        SchemaDomain::Maps,
        SchemaDomain::Media,
        SchemaDomain::Messages,
        SchemaDomain::Notes,
        SchemaDomain::Photos,
        SchemaDomain::Reminders,
    ];

    /// The lowercase name of the domain.
    pub fn name(self) -> &'static str {
        match self {
            SchemaDomain::Browser => "browser",
            SchemaDomain::Calendar => "calendar",
            SchemaDomain::Camera => "camera",
            SchemaDomain::Contacts => "contacts",
            SchemaDomain::Documents => "documents",
            SchemaDomain::Files => "files",
            SchemaDomain::Mail => "mail",
            SchemaDomain::Maps => "maps",
            SchemaDomain::Media => "media",
            SchemaDomain::Messages => "messages",
            SchemaDomain::Notes => "notes",
            SchemaDomain::Photos => "photos",
            SchemaDomain::Reminders => "reminders",
        }
    }

    pub fn parse(text: &str) -> Option<SchemaDomain> {
        SchemaDomain::ALL.iter().copied().find(|domain| domain.name() == text)
    }
}

/// Which text field a presentation problem belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Title,
    Subtitle,
}

/// Why a presentation value is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresentationError {
    /// The field is empty once trimmed.
    Empty(Field),
    /// The field is longer than its limit; the length is in characters.
    TooLong { field: Field, length: usize },
    /// The field holds a control character such as a line break.
    ControlCharacter(Field),
    /// The symbol name is not lowercase ASCII letters, digits and single dots within [`MAX_SYMBOL`] bytes.
    InvalidSymbol,
}

/// How an intent's entity or result is presented: a required title, an optional subtitle and symbol, and an optional
/// domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presentation {
    title: String,
    subtitle: Option<String>,
    symbol: Option<String>,
    domain: Option<SchemaDomain>,
}

impl Presentation {
    pub fn new(title: &str) -> Result<Presentation, PresentationError> {
        Ok(Presentation { title: clean(title, Field::Title, MAX_TITLE)?, subtitle: None, symbol: None, domain: None })
    }

    pub fn with_subtitle(mut self, subtitle: &str) -> Result<Presentation, PresentationError> {
        self.subtitle = Some(clean(subtitle, Field::Subtitle, MAX_SUBTITLE)?);
        Ok(self)
    }

    pub fn with_symbol(mut self, symbol: &str) -> Result<Presentation, PresentationError> {
        if !valid_symbol(symbol) {
            return Err(PresentationError::InvalidSymbol);
        }
        self.symbol = Some(symbol.to_string());
        Ok(self)
    }

    pub fn in_domain(mut self, domain: SchemaDomain) -> Presentation {
        self.domain = Some(domain);
        self
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    pub fn domain(&self) -> Option<SchemaDomain> {
        self.domain
    }
}

/// Trims a text field and checks its length and characters.
fn clean(text: &str, field: Field, limit: usize) -> Result<String, PresentationError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(PresentationError::Empty(field));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(PresentationError::ControlCharacter(field));
    }
    let length = trimmed.chars().count();
    if length > limit {
        return Err(PresentationError::TooLong { field, length });
    }
    Ok(trimmed.to_string())
}

fn valid_symbol(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SYMBOL
        && !name.starts_with('.')
        && !name.ends_with('.')
        && !name.contains("..")
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.')
}
