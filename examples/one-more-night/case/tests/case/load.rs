use std::path::PathBuf;

use one_more_night_case::{
    Ask, Case, CaseError, CaseFiles, Catalog, DoorState, GameTime, Interaction, Phase, Problem,
    Role, Zone,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
}

fn catalog() -> Catalog {
    let manifest = std::fs::read_to_string(fixture_dir().join("level_manifest.ron")).unwrap();
    Catalog::from_level_manifest(&manifest)
        .unwrap()
        .with_prop("prop_a")
        .with_prop("prop_b")
        .with_prop("prop_key")
        .with_portrait("portrait_01")
        .with_portrait("portrait_02")
        .with_activity("sit")
        .with_activity("stand")
}

fn files() -> CaseFiles {
    CaseFiles::read_dir(&fixture_dir().join("case")).unwrap()
}

fn time(text: &str) -> GameTime {
    GameTime::parse(text).unwrap()
}

#[test]
fn the_fixture_case_loads_with_every_file_resolved_against_the_catalog() {
    let case = Case::load_dir(&fixture_dir().join("case"), &catalog()).unwrap();
    let catalog = catalog();
    let fact = |id| case.facts.id(id).unwrap();
    let npc = |id| case.characters.id(id).unwrap();

    assert_eq!(case.strings.get(case.title).as_str(), "Placeholder case");
    assert_eq!(case.phase_at(time("20:59")), Phase::Before);
    assert_eq!(case.phase_at(time("21:00")), Phase::After);

    let character = case.characters.get(npc("npc_b"));
    assert_eq!(case.strings.name(character.name), "npc_b.name");
    assert_eq!(catalog.portraits.name(character.portrait), "portrait_02");
    assert_eq!(character.questioned_by, Role::Staff);

    let schedule = case.schedule(npc("npc_a")).unwrap();
    let entry = &schedule.entries[1];
    assert_eq!(schedule.entries[0].start, time("18:00"));
    assert_eq!(catalog.anchors.name(entry.anchor), "room_b_01");
    assert_eq!(catalog.activities.name(entry.activity), "stand");
    assert_eq!(entry.facing, Some(90.0));
    assert_eq!(entry.phase, Phase::Before);
    assert!(entry.critical);
    assert_eq!(entry.conversation, case.conversations.id("conv_1"));
    let ringer = case.schedule(npc("npc_b")).unwrap();
    assert_eq!(ringer.entries[0].rings, Some(vec![fact("f_2")]));
    assert_eq!(ringer.entries[1].rings, None);

    let conversation = case
        .conversations
        .get(case.conversations.id("conv_1").unwrap());
    assert!(conversation.private);
    assert_eq!(conversation.earshot, Some(4.0));
    assert_eq!(case.characters.name(conversation.lines[1].speaker), "npc_b");
    assert_eq!(conversation.lines[1].grants, vec![fact("f_2")]);
    assert_eq!(conversation.cover.len(), 1);

    let dialogue = case.dialogue(npc("npc_a")).unwrap();
    assert_eq!(case.strings.name(dialogue.brush_off), "npc_a.busy");
    assert_eq!(
        dialogue.topics[0].ask,
        Ask::Question(case.strings.id("npc_a.q1").unwrap())
    );
    assert_eq!(dialogue.topics[1].ask, Ask::Fact(fact("f_1")));
    assert_eq!(dialogue.topics[2].ask, Ask::Person(npc("npc_b")));
    assert_eq!(
        dialogue.topics[3].ask,
        Ask::Item(case.placements.id("p_5").unwrap())
    );
    let guarded = &dialogue.topics[0].responses[0];
    let window = guarded.when.window.unwrap();
    assert_eq!(window.from, time("20:00"));
    assert_eq!(window.until, GameTime::MIDNIGHT);
    assert_eq!(guarded.when.phase, Some(Phase::Before));
    assert_eq!(guarded.when.knows, vec![fact("f_1")]);
    assert_eq!(guarded.grants, vec![fact("f_3")]);
    assert!(case.dialogue(npc("npc_b")).is_none());

    assert_eq!(
        case.facts.get(fact("f_1")).object,
        case.placements.id("p_1")
    );
    let heard = case.facts.get(fact("f_2"));
    assert_eq!(heard.people.len(), 2);
    assert_eq!(catalog.rooms.name(heard.place.unwrap()), "room_b");
    assert_eq!(heard.time, Some(time("19:30")));
    assert_eq!(heard.unlocks_terms, vec![case.terms.id("t_room").unwrap()]);

    let deduction = case.deductions.get(case.deductions.id("d_1").unwrap());
    assert_eq!(case.strings.name(deduction.template), "d_1.template");
    assert_eq!(
        case.term_banks.name(deduction.blanks[1].bank),
        "bank_places"
    );
    assert_eq!(case.terms.name(deduction.blanks[1].answer), "t_room");
    assert_eq!(deduction.appears_with, vec![fact("f_1")]);
    assert!(deduction.prerequisite_for_finale);
    assert_eq!(deduction.unlocks, vec![case.deductions.id("d_2").unwrap()]);

    assert_eq!(
        catalog.anchors.name(case.finale.staging[1].anchor),
        "room_a_03"
    );
    let statement = &case.finale.statements[0];
    assert_eq!(case.deductions.name(statement.deduction), "d_2");
    assert_eq!(case.characters.name(statement.reaction[0].speaker), "npc_b");
    assert_eq!(case.strings.name(case.finale.ending[0]), "finale.end");

    let placement = |id| case.placements.get(case.placements.id(id).unwrap());
    let inspected = placement("p_1");
    assert_eq!(catalog.props.name(inspected.prop), "prop_a");
    assert_eq!(catalog.anchors.name(inspected.anchor), "room_a_04");
    let Interaction::Inspect { hotspots } = &inspected.interaction else {
        panic!("p_1 is not inspected");
    };
    assert_eq!(hotspots[0].grants, vec![fact("f_1")]);
    let Interaction::Document { pages, .. } = &placement("p_2").interaction else {
        panic!("p_2 is not a document");
    };
    assert_eq!(case.strings.name(pages[0]), "p_2.page");
    assert_eq!(
        placement("p_3").interaction,
        Interaction::Carry {
            key: catalog.keys.id("k_service")
        }
    );
    assert_eq!(placement("p_4").interaction, Interaction::Scenery);

    let service = catalog.doors.id("d_service").unwrap();
    assert_eq!(case.doors.staff_opens, vec![service]);
    let locked = case
        .doors
        .override_for(catalog.doors.id("d_ab").unwrap())
        .unwrap();
    assert_eq!(locked.state, Some(DoorState::Locked));
    assert_eq!(locked.key, catalog.keys.id("k_service"));
    assert_eq!(locked.zone, Some(Zone::Private));
    assert_eq!(locked.locked[0].from, time("23:00"));
    assert_eq!(
        case.doors.override_for(service).unwrap().enabled,
        Some(false)
    );

    assert!(case.features.is_on(catalog.features.id("F02").unwrap()));
    assert!(!case.features.is_on(catalog.features.id("F01").unwrap()));

    let debug = format!("{case:?}");
    assert!(debug.contains("case.title"));
    assert!(!debug.contains("Placeholder"));
    assert!(!debug.contains("Person A"));
}

enum Refused {
    With(CaseError),
    Syntax(&'static str),
}

struct Refusal {
    file: &'static str,
    from: &'static str,
    to: &'static str,
    refused: Refused,
}

fn invalid(file: &'static str, owner: &str, problem: Problem) -> Refused {
    Refused::With(CaseError::Invalid {
        file,
        owner: owner.to_string(),
        problem,
    })
}

#[test]
fn a_case_that_breaks_a_rule_is_refused_naming_the_file_and_ids_and_never_its_text() {
    let refusals = [
        Refusal {
            file: "case",
            from: "version: 1",
            to: "version: 2",
            refused: Refused::With(CaseError::UnsupportedVersion {
                file: "case",
                found: 2,
            }),
        },
        Refusal {
            file: "facts",
            from: "unlocks_terms: [\"t_a\"]",
            to: "unlocks_terms: [\"t_zz\"]",
            refused: Refused::With(CaseError::Unknown {
                file: "facts",
                owner: "fact `f_1`".to_string(),
                kind: "term",
                id: "t_zz".to_string(),
            }),
        },
        Refusal {
            file: "facts",
            from: "id: \"f_2\"",
            to: "id: \"f_1\"",
            refused: Refused::With(CaseError::Duplicate {
                file: "facts",
                kind: "fact",
                id: "f_1".to_string(),
            }),
        },
        Refusal {
            file: "dialogue",
            from: "\"npc_a.q1.r1\"",
            to: "\"An unreadable secret.\"",
            refused: Refused::With(CaseError::NotAnId {
                file: "dialogue",
                owner: "dialogue `npc_a`".to_string(),
                kind: "string",
            }),
        },
        Refusal {
            file: "deductions",
            from: "answer: \"t_room\"",
            to: "answer: \"t_a\"",
            refused: invalid("deductions", "deduction `d_1`", Problem::AnswerNotInBank),
        },
        Refusal {
            file: "conversations",
            from: "participants: [\"npc_a\", \"npc_b\"]",
            to: "participants: [\"npc_a\"]",
            refused: invalid(
                "conversations",
                "conversation `conv_1`",
                Problem::SpeakerNotParticipant,
            ),
        },
        Refusal {
            file: "schedules",
            from: "Schedule(npc: \"npc_b\"",
            to: "Schedule(npc: \"npc_c\"",
            refused: invalid("schedules", "schedule `npc_c`", Problem::NotInConversation),
        },
        Refusal {
            file: "schedules",
            from: "start: \"19:30\", anchor: \"room_b_01\"",
            to: "start: \"17:00\", anchor: \"room_b_01\"",
            refused: invalid("schedules", "schedule `npc_a`", Problem::EntriesOutOfOrder),
        },
        Refusal {
            file: "conversations",
            from: "seconds: 60.0, grants",
            to: "seconds: -1.0, grants",
            refused: invalid(
                "conversations",
                "conversation `conv_1`",
                Problem::NegativeDuration,
            ),
        },
        Refusal {
            file: "schedules",
            from: "facing: 90.0",
            to: "facing: NaN",
            refused: invalid("schedules", "schedule `npc_a`", Problem::NumberOutOfRange),
        },
        Refusal {
            file: "strings",
            from: "\"It was ___.\"",
            to: "\"It was.\"",
            refused: invalid("deductions", "deduction `d_2`", Problem::BlankCountMismatch),
        },
        Refusal {
            file: "finale",
            from: "Staging(npc: \"npc_b\"",
            to: "Staging(npc: \"npc_a\"",
            refused: invalid("finale", "finale `finale`", Problem::StagedTwice),
        },
        Refusal {
            file: "features",
            from: "off: [\"F01\"]",
            to: "off: []",
            refused: invalid("features", "feature `F01`", Problem::FeatureNotSet),
        },
        Refusal {
            file: "terms",
            from: "Term(id: \"t_a\",",
            to: "Term(id: \"t_a\", note: 1,",
            refused: Refused::Syntax("terms"),
        },
        Refusal {
            file: "conversations",
            from: "seconds: 60.0",
            to: "seconds: \"unreadable secret\"",
            refused: Refused::Syntax("conversations"),
        },
        Refusal {
            file: "schedules",
            from: "\"19:30\"",
            to: "\"19:75\"",
            refused: Refused::Syntax("schedules"),
        },
        Refusal {
            file: "doors",
            from: "until: \"24:00\"",
            to: "until: \"22:00\"",
            refused: Refused::Syntax("doors"),
        },
    ];
    for refusal in refusals {
        let files = files();
        let text = files.text(refusal.file).unwrap();
        assert!(text.contains(refusal.from), "{}", refusal.from);
        let text = text.replacen(refusal.from, refusal.to, 1);
        let error = Case::load(&files.with_file(refusal.file, text), &catalog())
            .err()
            .unwrap_or_else(|| panic!("`{}` was not refused", refusal.to));

        match refusal.refused {
            Refused::With(expected) => assert_eq!(error, expected),
            Refused::Syntax(file) => assert!(
                matches!(error, CaseError::Syntax { file: found, .. } if found == file),
                "{error:?}"
            ),
        }
        assert!(!error.to_string().contains("secret"));
        assert!(!format!("{error:?}").contains("secret"));
    }
}
