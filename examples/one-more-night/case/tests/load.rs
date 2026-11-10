use std::path::PathBuf;

use one_more_night_case::{
    Ask, Case, CaseError, CaseFiles, Catalog, DoorState, FILES, GameTime, Interaction, Phase,
    Problem, Role, Zone,
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

fn load() -> Case {
    Case::load(&files(), &catalog()).unwrap()
}

fn load_with(file: &str, from: &str, to: &str) -> Result<Case, CaseError> {
    let files = files();
    let text = files.text(file).unwrap();
    assert!(text.contains(from), "{file}.ron has no `{from}`");
    let text = text.replacen(from, to, 1);
    Case::load(&files.with_file(file, text), &catalog())
}

fn time(text: &str) -> GameTime {
    GameTime::parse(text).unwrap()
}

#[test]
fn the_fixture_directory_loads() {
    let case = Case::load_dir(&fixture_dir().join("case"), &catalog()).unwrap();
    assert_eq!(case.strings.name(case.title), "case.title");
    assert_eq!(case.strings.get(case.title).as_str(), "Placeholder case");
    assert_eq!(case.characters.len(), 3);
    assert_eq!(case.facts.len(), 3);
    assert_eq!(case.deductions.len(), 2);
    assert_eq!(case.placements.len(), 5);
}

#[test]
fn characters_resolve_their_name_and_portrait() {
    let case = load();
    let catalog = catalog();
    let npc = case.characters.id("npc_b").unwrap();
    let character = case.characters.get(npc);
    assert_eq!(case.strings.name(character.name), "npc_b.name");
    assert_eq!(catalog.portraits.name(character.portrait), "portrait_02");
    assert_eq!(character.questioned_by, Role::Staff);
}

#[test]
fn schedule_entries_resolve_anchors_activities_and_conversations() {
    let case = load();
    let catalog = catalog();
    let npc = case.characters.id("npc_a").unwrap();
    let schedule = case.schedule(npc).unwrap();
    assert_eq!(schedule.entries.len(), 2);
    let first = &schedule.entries[0];
    assert_eq!(first.start, time("18:00"));
    assert_eq!(first.facing, None);
    assert!(!first.critical);
    assert_eq!(first.conversation, None);
    let second = &schedule.entries[1];
    assert_eq!(catalog.anchors.name(second.anchor), "room_b_01");
    assert_eq!(catalog.activities.name(second.activity), "stand");
    assert_eq!(second.facing, Some(90.0));
    assert_eq!(second.phase, Phase::Before);
    assert!(second.critical);
    assert_eq!(second.conversation, case.conversations.id("conv_1"));
}

#[test]
fn conversations_resolve_speakers_lines_and_grants() {
    let case = load();
    let conversation = case
        .conversations
        .get(case.conversations.id("conv_1").unwrap());
    assert!(conversation.private);
    assert_eq!(conversation.earshot, Some(4.0));
    assert_eq!(conversation.participants.len(), 2);
    let line = &conversation.lines[1];
    assert_eq!(case.characters.name(line.speaker), "npc_b");
    assert_eq!(case.strings.name(line.text), "conv_1.l2");
    assert_eq!(line.grants, vec![case.facts.id("f_2").unwrap()]);
    assert_eq!(conversation.cover.len(), 1);
}

#[test]
fn dialogue_resolves_topics_conditions_and_effects() {
    let case = load();
    let npc = case.characters.id("npc_a").unwrap();
    let dialogue = case.dialogue(npc).unwrap();
    assert_eq!(case.strings.name(dialogue.brush_off), "npc_a.busy");
    assert_eq!(dialogue.topics.len(), 3);
    let question = &dialogue.topics[0];
    assert_eq!(
        question.ask,
        Ask::Question(case.strings.id("npc_a.q1").unwrap())
    );
    let guarded = &question.responses[0];
    let window = guarded.when.window.unwrap();
    assert_eq!(window.from, time("20:00"));
    assert_eq!(window.until, GameTime::MIDNIGHT);
    assert_eq!(guarded.when.phase, Some(Phase::Before));
    assert_eq!(guarded.when.knows, vec![case.facts.id("f_1").unwrap()]);
    assert_eq!(guarded.grants, vec![case.facts.id("f_3").unwrap()]);
    let open = &question.responses[1];
    assert_eq!(open.when.window, None);
    assert!(open.when.knows.is_empty());
    assert_eq!(
        dialogue.topics[1].ask,
        Ask::Fact(case.facts.id("f_1").unwrap())
    );
    assert_eq!(
        dialogue.topics[2].ask,
        Ask::Person(case.characters.id("npc_b").unwrap())
    );
    assert!(
        case.dialogue(case.characters.id("npc_b").unwrap())
            .is_none()
    );
}

#[test]
fn facts_resolve_their_tags() {
    let case = load();
    let catalog = catalog();
    let seen = case.facts.get(case.facts.id("f_1").unwrap());
    assert_eq!(seen.object, case.placements.id("p_1"));
    assert_eq!(seen.time, None);
    let heard = case.facts.get(case.facts.id("f_2").unwrap());
    assert_eq!(heard.people.len(), 2);
    assert_eq!(catalog.rooms.name(heard.place.unwrap()), "room_b");
    assert_eq!(heard.time, Some(time("19:30")));
    assert_eq!(heard.unlocks_terms, vec![case.terms.id("t_room").unwrap()]);
}

#[test]
fn deductions_resolve_banks_answers_and_unlocks() {
    let case = load();
    let deduction = case.deductions.get(case.deductions.id("d_1").unwrap());
    assert_eq!(case.strings.name(deduction.template), "d_1.template");
    assert_eq!(deduction.blanks.len(), 2);
    assert_eq!(
        case.term_banks.name(deduction.blanks[1].bank),
        "bank_places"
    );
    assert_eq!(case.terms.name(deduction.blanks[1].answer), "t_room");
    assert_eq!(deduction.appears_with, vec![case.facts.id("f_1").unwrap()]);
    assert!(deduction.prerequisite_for_finale);
    assert_eq!(deduction.unlocks, vec![case.deductions.id("d_2").unwrap()]);
    let bank = case.term_banks.get(deduction.blanks[0].bank);
    assert_eq!(bank.terms.len(), 2);
}

#[test]
fn the_finale_resolves_staging_and_statements() {
    let case = load();
    let catalog = catalog();
    assert_eq!(case.finale.staging.len(), 2);
    assert_eq!(
        catalog.anchors.name(case.finale.staging[1].anchor),
        "room_a_03"
    );
    let statement = &case.finale.statements[0];
    assert_eq!(case.deductions.name(statement.deduction), "d_2");
    assert_eq!(case.characters.name(statement.reaction[0].speaker), "npc_b");
    assert_eq!(case.strings.name(case.finale.ending[0]), "finale.end");
}

#[test]
fn placements_resolve_props_anchors_and_interactions() {
    let case = load();
    let catalog = catalog();
    let inspected = case.placements.get(case.placements.id("p_1").unwrap());
    assert_eq!(catalog.props.name(inspected.prop), "prop_a");
    assert_eq!(catalog.anchors.name(inspected.anchor), "room_a_04");
    let Interaction::Inspect { hotspots } = &inspected.interaction else {
        panic!("p_1 is not inspected");
    };
    assert_eq!(hotspots[0].grants, vec![case.facts.id("f_1").unwrap()]);
    let document = case.placements.get(case.placements.id("p_2").unwrap());
    let Interaction::Document { pages, grants } = &document.interaction else {
        panic!("p_2 is not a document");
    };
    assert_eq!(case.strings.name(pages[0]), "p_2.page");
    assert!(grants.is_empty());
    let carried = case.placements.get(case.placements.id("p_3").unwrap());
    assert_eq!(
        carried.interaction,
        Interaction::Carry {
            key: catalog.keys.id("k_service")
        }
    );
    let scenery = case.placements.get(case.placements.id("p_4").unwrap());
    assert_eq!(scenery.interaction, Interaction::Scenery);
    let item = case.placements.get(case.placements.id("p_5").unwrap());
    assert_eq!(item.interaction, Interaction::Carry { key: None });
}

#[test]
fn doors_resolve_the_key_ring_and_overrides() {
    let case = load();
    let catalog = catalog();
    let service = catalog.doors.id("d_service").unwrap();
    assert_eq!(case.doors.staff_opens, vec![service]);
    let locked = case
        .doors
        .override_for(catalog.doors.id("d_ab").unwrap())
        .unwrap();
    assert_eq!(locked.state, Some(DoorState::Locked));
    assert_eq!(locked.key, catalog.keys.id("k_service"));
    assert_eq!(locked.zone, Some(Zone::Private));
    assert_eq!(locked.enabled, None);
    assert_eq!(locked.locked[0].from, time("23:00"));
    let disabled = case.doors.override_for(service).unwrap();
    assert_eq!(disabled.enabled, Some(false));
    assert_eq!(disabled.state, None);
}

#[test]
fn every_file_is_required() {
    for file in FILES {
        let mut kept = CaseFiles::new();
        let all = files();
        for other in FILES {
            if other != file {
                kept = kept.with_file(other, all.text(other).unwrap());
            }
        }
        assert_eq!(
            Case::load(&kept, &catalog()).err(),
            Some(CaseError::MissingFile { file })
        );
    }
}

#[test]
fn another_schema_version_is_refused() {
    assert_eq!(
        load_with("case", "version: 1", "version: 2").err(),
        Some(CaseError::UnsupportedVersion {
            file: "case",
            found: 2
        })
    );
}

#[test]
fn a_dangling_reference_names_the_file_and_the_ids() {
    let error = load_with(
        "facts",
        "unlocks_terms: [\"t_a\"]",
        "unlocks_terms: [\"t_zz\"]",
    )
    .err()
    .unwrap();
    assert_eq!(
        error,
        CaseError::Unknown {
            file: "facts",
            owner: "fact `f_1`".to_string(),
            kind: "term",
            id: "t_zz".to_string(),
        }
    );
    assert_eq!(
        error.to_string(),
        "facts.ron: fact `f_1` refers to unknown term `t_zz`"
    );
}

#[test]
fn a_missing_string_is_a_dangling_reference() {
    let error = load_with("characters", "\"npc_a.name\"", "\"npc_a.nome\"")
        .err()
        .unwrap();
    assert_eq!(
        error,
        CaseError::Unknown {
            file: "characters",
            owner: "character `npc_a`".to_string(),
            kind: "string",
            id: "npc_a.nome".to_string(),
        }
    );
}

#[test]
fn an_id_missing_from_the_catalog_is_a_dangling_reference() {
    let error = load_with("schedules", "\"room_a_01\"", "\"room_z_01\"")
        .err()
        .unwrap();
    assert_eq!(
        error,
        CaseError::Unknown {
            file: "schedules",
            owner: "schedule `npc_a`".to_string(),
            kind: "anchor",
            id: "room_z_01".to_string(),
        }
    );
}

#[test]
fn a_duplicate_id_is_refused() {
    assert_eq!(
        load_with("facts", "id: \"f_2\"", "id: \"f_1\"").err(),
        Some(CaseError::Duplicate {
            file: "facts",
            kind: "fact",
            id: "f_1".to_string(),
        })
    );
    assert_eq!(
        load_with("strings", "\"npc_b.name\":", "\"npc_a.name\":").err(),
        Some(CaseError::Duplicate {
            file: "strings",
            kind: "string",
            id: "npc_a.name".to_string(),
        })
    );
    assert_eq!(
        load_with(
            "schedules",
            "Schedule(npc: \"npc_b\"",
            "Schedule(npc: \"npc_a\""
        )
        .err(),
        Some(CaseError::Duplicate {
            file: "schedules",
            kind: "schedule",
            id: "npc_a".to_string(),
        })
    );
}

#[test]
fn an_answer_outside_its_bank_is_refused() {
    assert_eq!(
        load_with("deductions", "answer: \"t_room\"", "answer: \"t_a\"").err(),
        Some(CaseError::Invalid {
            file: "deductions",
            owner: "deduction `d_1`".to_string(),
            problem: Problem::AnswerNotInBank,
        })
    );
}

#[test]
fn a_speaker_outside_the_conversation_is_refused() {
    assert_eq!(
        load_with(
            "conversations",
            "participants: [\"npc_a\", \"npc_b\"]",
            "participants: [\"npc_a\"]"
        )
        .err(),
        Some(CaseError::Invalid {
            file: "conversations",
            owner: "conversation `conv_1`".to_string(),
            problem: Problem::SpeakerNotParticipant,
        })
    );
}

#[test]
fn an_unknown_field_is_refused() {
    let error = load_with("terms", "Term(id: \"t_a\",", "Term(id: \"t_a\", note: 1,")
        .err()
        .unwrap();
    assert!(matches!(error, CaseError::Syntax { file: "terms", .. }));
}

#[test]
fn a_syntax_error_gives_a_position_and_no_file_content() {
    let error = load_with(
        "conversations",
        "seconds: 60.0",
        "seconds: \"unreadable secret\"",
    )
    .err()
    .unwrap();
    let CaseError::Syntax { file, line, .. } = error else {
        panic!("not a syntax error: {error}");
    };
    assert_eq!(file, "conversations");
    assert_eq!(line, 8);
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
}

#[test]
fn a_malformed_time_is_a_syntax_error() {
    let error = load_with("schedules", "\"19:30\"", "\"19:75\"")
        .err()
        .unwrap();
    assert!(matches!(
        error,
        CaseError::Syntax {
            file: "schedules",
            ..
        }
    ));
}

#[test]
fn debug_output_holds_no_string_text() {
    let debug = format!("{:?}", load());
    assert!(debug.contains("case.title"));
    assert!(!debug.contains("Placeholder"));
    assert!(!debug.contains("Person A"));
}

#[test]
fn the_manor_manifest_fills_a_catalog() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/manor/export/level_manifest.ron");
    let catalog = Catalog::from_level_manifest(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(catalog.rooms.id("hall").is_some());
    assert!(catalog.doors.id("d_front").is_some());
    assert!(catalog.anchors.id("pantry_03").is_some());
    assert!(catalog.keys.id("k_cellar").is_some());
    assert_eq!(catalog.features.len(), 14);
}

fn manifest_with(from: &str, to: &str) -> Result<Catalog, CaseError> {
    let text = std::fs::read_to_string(fixture_dir().join("level_manifest.ron")).unwrap();
    assert!(text.contains(from), "the manifest has no `{from}`");
    Catalog::from_level_manifest(&text.replacen(from, to, 1))
}

fn invalid(file: &'static str, owner: &str, problem: Problem) -> Option<CaseError> {
    Some(CaseError::Invalid {
        file,
        owner: owner.to_string(),
        problem,
    })
}

#[test]
fn text_where_an_id_belongs_is_refused_without_being_shown() {
    let error = load_with("dialogue", "\"npc_a.q1.r1\"", "\"An unreadable secret.\"")
        .err()
        .unwrap();
    assert_eq!(
        error,
        CaseError::NotAnId {
            file: "dialogue",
            owner: "dialogue `npc_a`".to_string(),
            kind: "string",
        }
    );
    assert!(!error.to_string().contains("secret"));

    let error = load_with("facts", "id: \"f_2\"", "id: \"An unreadable secret.\"")
        .err()
        .unwrap();
    assert_eq!(
        error,
        CaseError::NotAnId {
            file: "facts",
            owner: "fact number 2".to_string(),
            kind: "fact",
        }
    );

    let error = load_with(
        "schedules",
        "Schedule(npc: \"npc_a\"",
        "Schedule(npc: \"An unreadable secret.\"",
    )
    .err()
    .unwrap();
    assert!(!format!("{error} {error:?}").contains("secret"));
}

#[test]
fn a_second_dialogue_for_one_character_is_refused() {
    let files = files();
    let text = files.text("dialogue").unwrap().trim_end();
    let text = format!(
        "{}Dialogue(npc: \"npc_a\", brush_off: \"npc_a.busy\", no_answer: \"npc_a.no_answer\", topics: [])]",
        text.strip_suffix(']').unwrap()
    );
    assert_eq!(
        Case::load(&files.with_file("dialogue", text), &catalog()).err(),
        Some(CaseError::Duplicate {
            file: "dialogue",
            kind: "dialogue",
            id: "npc_a".to_string(),
        })
    );
}

#[test]
fn a_second_override_for_one_door_is_refused() {
    assert_eq!(
        load_with(
            "doors",
            "DoorOverride(door: \"d_service\"",
            "DoorOverride(door: \"d_ab\""
        )
        .err(),
        Some(CaseError::Duplicate {
            file: "doors",
            kind: "door override",
            id: "d_ab".to_string(),
        })
    );
}

#[test]
fn a_schedule_cannot_name_a_conversation_of_other_characters() {
    assert_eq!(
        load_with(
            "schedules",
            "Schedule(npc: \"npc_b\"",
            "Schedule(npc: \"npc_c\""
        )
        .err(),
        invalid("schedules", "schedule `npc_c`", Problem::NotInConversation)
    );
}

#[test]
fn entries_of_one_phase_are_in_order_of_start() {
    assert_eq!(
        load_with(
            "schedules",
            "start: \"19:30\", anchor: \"room_b_01\"",
            "start: \"17:00\", anchor: \"room_b_01\""
        )
        .err(),
        invalid("schedules", "schedule `npc_a`", Problem::EntriesOutOfOrder)
    );
}

#[test]
fn a_negative_duration_is_refused() {
    assert_eq!(
        load_with(
            "conversations",
            "seconds: 60.0, grants",
            "seconds: -1.0, grants"
        )
        .err(),
        invalid(
            "conversations",
            "conversation `conv_1`",
            Problem::NegativeDuration
        )
    );
}

#[test]
fn a_template_has_one_gap_for_each_blank() {
    assert_eq!(
        load_with("strings", "\"It was ___.\"", "\"It was.\"").err(),
        invalid("deductions", "deduction `d_2`", Problem::BlankCountMismatch)
    );
}

#[test]
fn a_character_is_staged_once() {
    assert_eq!(
        load_with("finale", "Staging(npc: \"npc_b\"", "Staging(npc: \"npc_a\"").err(),
        invalid("finale", "finale `finale`", Problem::StagedTwice)
    );
}

#[test]
fn a_window_that_ends_before_it_starts_is_a_syntax_error() {
    let error = load_with("doors", "until: \"24:00\"", "until: \"22:00\"")
        .err()
        .unwrap();
    assert!(matches!(error, CaseError::Syntax { file: "doors", .. }));
}

#[test]
fn features_resolve_to_the_ones_turned_on() {
    let case = load();
    let catalog = catalog();
    assert!(case.features.is_on(catalog.features.id("F02").unwrap()));
    assert!(!case.features.is_on(catalog.features.id("F01").unwrap()));
}

#[test]
fn every_feature_of_the_level_is_set_exactly_once() {
    assert_eq!(
        load_with("features", "off: [\"F01\"]", "off: []").err(),
        invalid("features", "feature `F01`", Problem::FeatureNotSet)
    );
    assert_eq!(
        load_with("features", "off: [\"F01\"]", "off: [\"F01\", \"F02\"]").err(),
        Some(CaseError::Duplicate {
            file: "features",
            kind: "feature setting",
            id: "F02".to_string(),
        })
    );
    assert!(matches!(
        load_with("features", "off: [\"F01\"]", "off: [\"F01\", \"F99\"]").err(),
        Some(CaseError::Unknown {
            kind: "feature",
            ..
        })
    ));
}

#[test]
fn a_level_manifest_with_a_repeated_id_or_another_version_is_refused() {
    assert_eq!(
        manifest_with("Room(id: \"room_b\"", "Room(id: \"room_a\"").err(),
        Some(CaseError::Duplicate {
            file: "level_manifest",
            kind: "room",
            id: "room_a".to_string(),
        })
    );
    assert_eq!(
        manifest_with("version: 1", "version: 2").err(),
        Some(CaseError::UnsupportedVersion {
            file: "level_manifest",
            found: 2,
        })
    );
    assert!(matches!(
        manifest_with("rooms: [", "rooms: (").err(),
        Some(CaseError::Syntax {
            file: "level_manifest",
            ..
        })
    ));
}
