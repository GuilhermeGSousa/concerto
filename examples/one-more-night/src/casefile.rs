use concerto::ecs::Resource;

use crate::case::{Case, CaseError, CaseFiles, Catalog, FILES, StrId};
use crate::level::Level;

/// The props, portraits and activities the game offers a case.
pub const PROPS: [&str; 6] = [
    "prop_case",
    "prop_note",
    "prop_basket",
    "prop_key",
    "prop_cup",
    "prop_cufflink",
];
pub const PORTRAITS: [&str; 10] = [
    "portrait_01",
    "portrait_02",
    "portrait_03",
    "portrait_04",
    "portrait_05",
    "portrait_06",
    "portrait_07",
    "portrait_08",
    "portrait_09",
    "portrait_10",
];
pub const ACTIVITIES: [&str; 2] = ["sit", "stand"];

/// The case being played, the ids it borrows, and the level as it ships.
#[derive(Resource)]
pub struct CaseData {
    pub case: Case,
    pub catalog: Catalog,
    pub level: Level,
}

macro_rules! test_file {
    ($name:literal) => {
        include_str!(concat!("../cases/test/", $name, ".ron"))
    };
}

impl CaseData {
    /// Loads a case from the texts of its files and its level's manifest.
    pub fn load(files: &CaseFiles, manifest: &str) -> Result<Self, CaseError> {
        let mut catalog = Catalog::from_level_manifest(manifest)?;
        for prop in PROPS {
            catalog = catalog.with_prop(prop);
        }
        for portrait in PORTRAITS {
            catalog = catalog.with_portrait(portrait);
        }
        for activity in ACTIVITIES {
            catalog = catalog.with_activity(activity);
        }
        let level = Level::from_manifest(manifest, &catalog)?;
        let case = Case::load(files, &catalog)?;
        Ok(Self {
            case,
            catalog,
            level,
        })
    }

    /// The open test case, built into the game.
    pub fn test() -> Self {
        let texts = [
            test_file!("case"),
            test_file!("strings"),
            test_file!("characters"),
            test_file!("schedules"),
            test_file!("conversations"),
            test_file!("dialogue"),
            test_file!("facts"),
            test_file!("terms"),
            test_file!("deductions"),
            test_file!("finale"),
            test_file!("placements"),
            test_file!("doors"),
            test_file!("features"),
        ];
        let names = [
            "case",
            "strings",
            "characters",
            "schedules",
            "conversations",
            "dialogue",
            "facts",
            "terms",
            "deductions",
            "finale",
            "placements",
            "doors",
            "features",
        ];
        debug_assert!(names.iter().all(|name| FILES.contains(name)));
        let files = names
            .into_iter()
            .zip(texts)
            .fold(CaseFiles::new(), |files, (name, text)| {
                files.with_file(name, text)
            });
        Self::load(&files, test_file!("level_manifest")).expect("the open test case loads")
    }

    /// An empty case over the manor, for walking the house.
    pub fn manor_walk() -> Self {
        let files = [
            ("case", include_str!("../cases/manor_walk/case.ron")),
            ("strings", include_str!("../cases/manor_walk/strings.ron")),
            (
                "characters",
                include_str!("../cases/manor_walk/characters.ron"),
            ),
            (
                "schedules",
                include_str!("../cases/manor_walk/schedules.ron"),
            ),
            (
                "conversations",
                include_str!("../cases/manor_walk/conversations.ron"),
            ),
            ("dialogue", include_str!("../cases/manor_walk/dialogue.ron")),
            ("facts", include_str!("../cases/manor_walk/facts.ron")),
            ("terms", include_str!("../cases/manor_walk/terms.ron")),
            (
                "deductions",
                include_str!("../cases/manor_walk/deductions.ron"),
            ),
            ("finale", include_str!("../cases/manor_walk/finale.ron")),
            (
                "placements",
                include_str!("../cases/manor_walk/placements.ron"),
            ),
            ("doors", include_str!("../cases/manor_walk/doors.ron")),
            ("features", include_str!("../cases/manor_walk/features.ron")),
        ]
        .into_iter()
        .fold(CaseFiles::new(), |files, file| {
            files.with_file(file.0, file.1)
        });
        Self::load(
            &files,
            include_str!("../assets/manor/export/level_manifest.ron"),
        )
        .expect("the manor walk loads")
    }

    /// A case over the manor's level manifest.
    pub fn manor(files: &CaseFiles) -> Result<Self, CaseError> {
        Self::load(
            files,
            include_str!("../assets/manor/export/level_manifest.ron"),
        )
    }

    /// The text of a string of the case.
    pub fn text(&self, id: StrId) -> &str {
        self.case.strings.get(id).as_str()
    }
}

/// What the casebook calls a room of the level.
pub fn room_title(id: &str) -> &str {
    match id {
        "room_a" => "Drawing room",
        "room_b" => "Study",
        "corridor" => "Service corridor",
        other => other,
    }
}
