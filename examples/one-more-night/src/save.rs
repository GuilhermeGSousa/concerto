use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::book::Casebook;
use crate::case::Role;
use crate::casefile::CaseData;
use crate::session::Session;

/// The version of the save file's layout.
pub const SAVE_VERSION: u32 = 1;
const SLOT: &str = "one-more-night.ron";
const BACKUPS: usize = 2;

/// What survives closing the game: ids, never text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SaveFile {
    pub version: u32,
    pub facts: Vec<String>,
    pub confirmed: Vec<String>,
    pub loops: u32,
    /// The first player is the Staff.
    pub roles_swapped: bool,
}

/// A casebook and roles read back from a save.
pub struct Restored {
    pub book: Casebook,
    pub roles: [Role; 2],
}

impl SaveFile {
    /// What to save of the game as it stands at a loop boundary.
    pub fn of(book: &Casebook, session: &Session, data: &CaseData) -> Self {
        Self {
            version: SAVE_VERSION,
            facts: book
                .facts()
                .iter()
                .map(|fact| data.case.facts.name(*fact).to_string())
                .collect(),
            confirmed: book
                .confirmed()
                .iter()
                .map(|deduction| data.case.deductions.name(*deduction).to_string())
                .collect(),
            loops: book.loops(),
            roles_swapped: session.roles[0] == Role::Staff,
        }
    }

    /// The casebook and roles this save holds; an id the case no longer has is dropped.
    pub fn restore(&self, data: &CaseData) -> Restored {
        let facts = self
            .facts
            .iter()
            .filter_map(|name| data.case.facts.id(name))
            .collect();
        let confirmed = self
            .confirmed
            .iter()
            .filter_map(|name| data.case.deductions.id(name))
            .collect();
        Restored {
            book: Casebook::restored(facts, confirmed, self.loops),
            roles: if self.roles_swapped {
                [Role::Staff, Role::Guest]
            } else {
                [Role::Guest, Role::Staff]
            },
        }
    }

    fn slot(dir: &Path, backup: usize) -> PathBuf {
        match backup {
            0 => dir.join(SLOT),
            backup => dir.join(format!("{SLOT}.{backup}")),
        }
    }

    /// Writes the save into `dir`, keeping the two before it as backups.
    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        for backup in (0..BACKUPS).rev() {
            let from = Self::slot(dir, backup);
            if from.exists() {
                std::fs::rename(from, Self::slot(dir, backup + 1))?;
            }
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(std::io::Error::other)?;
        std::fs::write(Self::slot(dir, 0), text)
    }

    /// Reads the newest save in `dir` that parses and has this version.
    pub fn read(dir: &Path) -> Option<Self> {
        (0..=BACKUPS).find_map(|backup| {
            let text = std::fs::read_to_string(Self::slot(dir, backup)).ok()?;
            let save: SaveFile = ron::from_str(&text).ok()?;
            (save.version == SAVE_VERSION).then_some(save)
        })
    }
}

/// Where saves go: `ONE_MORE_NIGHT_SAVES`, or `saves` under the working directory.
pub fn save_dir() -> PathBuf {
    std::env::var_os("ONE_MORE_NIGHT_SAVES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("saves"))
}

#[cfg(test)]
mod tests {
    use super::SaveFile;
    use crate::book::Casebook;
    use crate::case::Role;
    use crate::casefile::CaseData;
    use crate::session::Session;

    #[test]
    fn a_save_holds_ids_restores_them_and_falls_back_to_a_backup() {
        let data = CaseData::test();
        let mut book = Casebook::default();
        book.grant(data.case.facts.id("f_02").unwrap());
        book.end_night();
        let mut session = Session::default();
        session.roles.swap(0, 1);
        let dir = std::env::temp_dir().join(format!("omn-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let first = SaveFile::of(&book, &session, &data);
        assert_eq!(first.facts, ["f_02"]);
        first.write(&dir).unwrap();
        book.grant(data.case.facts.id("f_05").unwrap());
        SaveFile::of(&book, &session, &data).write(&dir).unwrap();

        let restored = SaveFile::read(&dir).unwrap().restore(&data);
        assert_eq!(restored.book.facts().len(), 2);
        assert_eq!(restored.book.loops(), 1);
        assert_eq!(restored.roles, [Role::Staff, Role::Guest]);

        std::fs::write(dir.join("one-more-night.ron"), "not a save").unwrap();
        assert_eq!(
            SaveFile::read(&dir),
            Some(first),
            "a broken slot falls back"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
