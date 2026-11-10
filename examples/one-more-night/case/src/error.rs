use std::fmt;

/// Why a case or a level manifest did not load; it names files and ids, never text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseError {
    MissingFile {
        file: &'static str,
    },
    Unreadable {
        file: &'static str,
    },
    Syntax {
        file: &'static str,
        line: usize,
        column: usize,
    },
    UnsupportedVersion {
        file: &'static str,
        found: u32,
    },
    NotAnId {
        file: &'static str,
        owner: String,
        kind: &'static str,
    },
    Duplicate {
        file: &'static str,
        kind: &'static str,
        id: String,
    },
    Unknown {
        file: &'static str,
        owner: String,
        kind: &'static str,
        id: String,
    },
    Invalid {
        file: &'static str,
        owner: String,
        problem: Problem,
    },
}

/// A rule the files broke that is not a dangling reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    SpeakerNotParticipant,
    AnswerNotInBank,
    NotInConversation,
    EntriesOutOfOrder,
    NegativeDuration,
    BlankCountMismatch,
    StagedTwice,
    FeatureNotSet,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Problem::SpeakerNotParticipant => {
                "has a line spoken by someone who is not a participant"
            }
            Problem::AnswerNotInBank => "has a blank whose answer is not in its bank",
            Problem::NotInConversation => "names a conversation its character takes no part in",
            Problem::EntriesOutOfOrder => "has entries of one phase that are not in order of start",
            Problem::NegativeDuration => "has a line whose seconds are not zero or more",
            Problem::BlankCountMismatch => {
                "has a template whose `___` count is not its blank count"
            }
            Problem::StagedTwice => "stages a character twice",
            Problem::FeatureNotSet => "is neither on nor off",
        })
    }
}

impl fmt::Display for CaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaseError::MissingFile { file } => write!(f, "{file}.ron is missing"),
            CaseError::Unreadable { file } => write!(f, "{file}.ron could not be read"),
            CaseError::Syntax { file, line, column } => {
                write!(f, "{file}.ron:{line}:{column}: does not match the schema")
            }
            CaseError::UnsupportedVersion { file, found } => {
                write!(f, "{file}.ron: version {found} is not supported")
            }
            CaseError::NotAnId { file, owner, kind } => write!(
                f,
                "{file}.ron: {owner} has text where the id of a {kind} belongs"
            ),
            CaseError::Duplicate { file, kind, id } => {
                write!(f, "{file}.ron: {kind} `{id}` is defined twice")
            }
            CaseError::Unknown {
                file,
                owner,
                kind,
                id,
            } => write!(f, "{file}.ron: {owner} refers to unknown {kind} `{id}`"),
            CaseError::Invalid {
                file,
                owner,
                problem,
            } => write!(f, "{file}.ron: {owner} {problem}"),
        }
    }
}

impl std::error::Error for CaseError {}
