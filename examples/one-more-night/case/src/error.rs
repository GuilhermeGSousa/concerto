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
        found: u32,
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
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Problem::SpeakerNotParticipant => {
                "has a line spoken by someone who is not a participant"
            }
            Problem::AnswerNotInBank => "has a blank whose answer is not in its bank",
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
            CaseError::UnsupportedVersion { found } => write!(
                f,
                "case.ron: schema version {found} is not version {}",
                crate::FORMAT_VERSION
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
