# Test suite rewrite: task reports

## Task 3: ecs

STATUS: DONE WITH CONCERNS

COMMIT: (task 3 is 10 commits, oldest first)
- 9c843d0 Collapse the added and changed flag tests into one table
- e31a67c Turn the storage, filter and event tests into tables
- 8542845 Turn the required component tests into tables
- 8a3cbcf Turn the hierarchy tests into one table of edits
- b0fc899 Turn the schedule ordering tests into one table and share unit fixtures
- ff57857 Turn the lifecycle, skip and signal test families into tables
- e76e9cc Replace the reachability and entity id families with proptest properties
- d819110 Turn the executor panic and thread placement tests into tables
- 3fdf9ef Turn the scene component lookup, encoding and spawn families into tables
- c523184 Name the fields an ordering case observes instead of returning a tuple

On resume: the storage integration test was already written and committed in e31a67c. The only uncommitted work was the tuple fix in change_tick_tests.rs, which is now c523184.

GATE (CARGO_INCREMENTAL=0, concerto-ecs only):
- `cargo fmt --all -- --check`: pass
- `cargo clippy --locked -p concerto-ecs --all-targets -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings`: pass
- `cargo test --locked -p concerto-ecs`: pass (lib 75, integration binary 87, doctests 7 passed with 4 ignored and 1 ignored)

COUNTS: `#[test]` in crates/ecs went from 328 before to 162 after. The 162 includes the two `#[test]` functions inside the `proptest!` blocks.

CHANGED:
- crates/ecs/Cargo.toml: adds `proptest.workspace = true` as a dev-dependency.
- Cargo.lock: rewritten by `cargo metadata`. proptest and its dependencies are locked for the first time, because no crate used proptest before.
- crates/ecs/src/testing.rs: new crate-level `#[cfg(test)]` fixtures module. It holds Gate and GateOpen, moved from `system::input::test_support`, and the Outer/Inner schedule labels, which were duplicated in schedule.rs and multi_thread.rs.
- crates/ecs/src/lib.rs: registers the `testing` module. The test module is now a presence-filter test, a migration-keeps-ticks test, one get_entity test, and an event table.
- crates/ecs/src/change_tick_tests.rs:
  - `added_and_changed_follow_each_write` table (Ref, Mut, Res and ResMut over write, overwrite and reinsert steps).
  - One set_if_neq test covering both components and resources.
  - One check_change_ticks threshold test that also asserts on its listener.
  - The tuple log types are gone.
- crates/ecs/src/component/required.rs: three tables (listing, cycle panics, spawn/insert outcomes). `write_skips_a_column_that_already_has_a_value` and `added_matches_a_required_component` are kept.
- crates/ecs/src/entity/hierarchy.rs: the test module is removed and its tests moved to tests/ecs/hierarchy.rs.
- crates/ecs/src/entity/mod.rs: an id-string round-trip proptest, plus one test for the format and the malformed ids.
- crates/ecs/src/system/reachability.rs: one proptest checking try_add_edge against a search over the accepted edges.
- crates/ecs/src/system/schedule.rs: keeps only the stood-in initialisation test, the stand-in access/sync-point test, and the run_schedule reentrancy test.
- crates/ecs/src/system/executor/single_thread.rs: the test module is removed.
- crates/ecs/src/system/executor/multi_thread.rs:
  - A panic table and a calling-thread table.
  - The `println!` is removed.
  - Uses the shared Outer/Inner labels.
- crates/ecs/src/system/input.rs: `test_support` removed (moved to testing.rs).
- crates/ecs/src/system/mod.rs: imports Gate from `crate::testing`.
- crates/ecs/src/resource.rs: four tests become two (optional values, optional access).
- crates/ecs/src/query/filter.rs: two tests become one.
- crates/ecs/src/system/input/input_set.rs: the "another marker" assertion is merged into the union test.
- crates/ecs/tests/ecs/storage.rs: new. One table of structural changes that checks the rows, the drop count, and the query membership.
- crates/ecs/tests/ecs/hierarchy.rs: replaces hierarchy_despawn.rs. It has:
  - an edit table checking Children and ChildOf
  - a deep despawn test
  - a Changed<Children> scenario
- crates/ecs/tests/ecs/ordering.rs: one ordering table covering constraints, sets, chains, tag filters, sync points and data-disjoint systems, plus one cycle table.
- crates/ecs/tests/ecs/main.rs: adds `mod storage`; `mod hierarchy_despawn` is renamed to `mod hierarchy`.
- Other files in crates/ecs/tests/ecs/ that changed:
  - required_external.rs: a success table and a panic table.
  - required_lifecycle.rs: one table; its tuple fixture is replaced.
  - component_lifecycle.rs: a Tracked callback table using a `Firing` struct instead of tuples, and one merged on_replace test.
  - lifecycle_signals.rs: an operation table and a refusal table.
  - missing_resources.rs: a skip table and one test covering both executors.
  - validation_errors.rs: a `Record` struct replaces the tuple.
  - system_set.rs and resource_get_or_insert.rs: one test each.
  - signals.rs: the two queued-signal tests are merged.
  - component_registry.rs: a name lookup table.
  - reflect_component.rs: an entity-reference encoding table.
  - scene_component.rs: a reference spawn table.
  - component_reflection.rs: a listing table.
  - despawn_callbacks.rs and deferred_callbacks.rs: their hierarchy tests are moved out.

BREAK-AND-REVERT (each table or property was failed by one edit, then the edit was reverted):
- added_and_changed table: `Mut::is_added` reading the changed tick.
- ticks_survive_wrap_around (cover for a deleted test): skipping the resource tick clamp.
- storage table: dropping `set_location` for the swapped row, once in each of the despawn, insert and remove paths.
- event table: clearing the wrong buffer in `update`.
- Required listing: reversing the visit order.
- Required cycle table: dropping the type path from the panic.
- Required spawn table: letting deeper constructors override nearer ones.
- required_external success table: ignoring identical registrations.
- required_lifecycle table: reversing the requirement order.
- hierarchy table: never removing an empty Children.
- ordering table: flipping the conflict edge; and ignoring the exclude filter.
- cycle table: changing the panic text.
- Track table: despawn skipping on_remove.
- Lifecycle signal table: dropping the remove signal.
- Lifecycle refusal table: disabling the reads-world check.
- Skip table: unwrapping a missing ResMut.
- reachability property: no ancestor propagation in `add_edge`.
- id property: swapping index and generation in `to_id_string`.
- Executor thread table: spawning exclusive systems off the scope thread.
- Executor panic table: swallowing task panics.
- Name lookup table: returning true for an unregistered name.
- Entity-ref table: shifting the Index encoding by one.
- Spawn reference table: treating unset as a node index.
- Listing table: taking only one component.

DELETED (each with what still covers it):

change_tick_tests.rs
- `ref_reports_added_then_changed`, `mut_tells_added_from_changed`, `res_mut_tells_added_from_changed`, `res_is_added_once`, `get_resource_mut_stamps_a_change`, `overwriting_a_component_marks_it_changed_but_not_added`, `overwriting_a_resource_marks_it_changed_but_not_added`: rows of `added_and_changed_follow_each_write`.
- `set_if_neq_writes_only_a_different_value` and `res_mut_set_if_neq_writes_only_a_different_value`: the merged `set_if_neq_writes_only_a_different_value`.
- `check_change_ticks_does_nothing_below_the_threshold` and `a_check_change_ticks_listener_receives_the_check`: `check_change_ticks_runs_once_per_threshold_and_tells_its_listeners`.
- `resource_stamps_are_clamped_without_a_per_type_registry`: `ticks_survive_wrap_around`, which fails when resource clamping is broken.
- `ref_derefs_to_the_component`: trivia (Deref).

lib.rs
- `despawn_drops_components`, `migration_moves_components_without_dropping_them`, `replacing_a_component_drops_the_old_value`, `removing_a_component_drops_it`, `bundle_insert_replacing_and_adding_drops_only_the_displaced_value`, `migrating_a_non_final_row_keeps_other_entities_intact`, `remove_last_component_leaves_empty_entity`, `insert_on_new_archetype`, `insert_on_existing_archetype`, `insert_twice`, `insert_bundle_replaces_existing_and_adds_new_component`: rows of storage.rs `structural_changes_keep_every_row_and_drop_each_value_once`.
- `test_query`, `test_added`, `test_or_query`, `test_add_children`: trivia (no assertions). Also covered by `presence_filters_select_entities_by_their_components`, `a_first_run_sees_everything_as_added`, and the hierarchy table.
- `spawn_despawn`: entity_store `reservations_reuse_freed_indices_last_freed_first_with_a_new_generation`.
- `spawn_from_system`: the "applied from commands" row of lifecycle_signals `each_operation_runs_its_hook_then_its_signal_while_the_value_is_readable`, and `a_command_applied_after_the_reader_is_seen_on_its_next_run`.
- `remove_resource`, `overwrite_resource`, `resource_system_read_write`: trivia. Also covered by the Res Reinsert/Overwrite rows of the stamp table (they assert the final value) and by `remove_resource` used in many tests.
- `remove_component_leaves_other_components` and `without_filter`: `presence_filters_select_entities_by_their_components` plus the storage "removing drops the component" row.
- `events_sent_and_read_same_frame`, `event_survives_one_buffer_swap`, `event_expires_after_two_buffer_swaps`, `an_event_is_never_read_twice`, `readers_of_the_same_event_have_independent_cursors`, `a_reader_that_skips_frames_resumes_from_the_oldest_buffered_event`, `event_written_after_a_read_is_visible_next_frame`: rows of `a_reader_sees_each_event_once_until_two_swaps_expire_it`.
- `added_filter_survives_insert` and `component_ticks_preserved_after_sibling_insert`: `a_migration_keeps_the_ticks_of_the_components_it_carries`.
- `get_entity_returns_none_for_missing_component` and `get_entity_on_an_entity_query_returns_none_for_a_despawned_entity`: `get_entity_finds_only_live_entities_the_query_matches`.

component/required.rs
- `unregistered_type_has_no_table`, `component_without_requirements_has_an_empty_table`, `direct_requirement_is_listed`, `requirements_are_flattened_deepest_first`, `duplicate_requirements_appear_once`, `requiring_a_type_registers_its_table`: `a_requirement_table_lists_every_requirement_once_deepest_first`.
- `cycle_panics_naming_the_types` and `self_requirement_panics`: `a_requirement_cycle_panics_naming_the_types`.
- `nearest_constructor_wins`: the "nearest constructor wins" row of `spawn_and_insert_fill_in_missing_requirements_and_keep_given_values`.
- `spawn_adds_a_default_required_component`, `spawn_adds_transitive_requirements`, `spawn_uses_the_declared_constructor`, `bundle_value_wins_in_either_order`, `insert_adds_a_default_required_component`, `insert_keeps_an_existing_required_component`, `insert_bundle_value_wins_over_the_default`, `removing_the_requirer_leaves_the_required_component`, `requirer_inserted_before_the_required_value_ends_with_the_stored_value`: rows of the same spawn/insert table.

tests/ecs/required_external.rs
- `external_registration_adds_a_requirement`, `_merges_with_the_types_own`, `_uses_the_given_constructor`, `_reaches_types_built_before_it`, `_reaches_types_built_after_it`, `_is_allowed_while_the_archetype_is_empty`, `a_declared_requirement_wins_over_an_external_one`, `the_later_external_registration_wins`, `identical_registration_is_a_no_op_while_an_entity_holds_the_type`: rows of `an_external_requirement_reaches_every_spawn_of_the_type`.
- `external_registration_panics_when_an_entity_holds_the_type`, `_when_an_entity_holds_a_requirer`, `_on_a_cycle`, `_on_a_self_requirement`, `different_constructor_still_panics_while_an_entity_holds_the_type`: rows of `an_external_requirement_that_would_change_live_entities_or_cycle_panics_naming_the_type`.

tests/ecs/required_lifecycle.rs
- All 7 tests: rows of `requirements_fire_their_hooks_and_signals_before_their_requirers`.

Hierarchy
- `src/entity/hierarchy.rs`: `attaching_a_child_changes_children` and `detaching_one_of_several_children_changes_children` are covered by hierarchy.rs `attaching_or_detaching_a_child_changes_the_parents_children`. `detaching_the_last_child_removes_children` is covered by the "detaching the last child" row of `edits_keep_children_and_child_of_in_step`.
- `hierarchy_despawn.rs`: `despawn_takes_descendants_and_detaches_the_surviving_parent`, `reparenting_repairs_the_previous_parents_children`, `inserting_child_of_reparents_without_going_through_the_builder`, `re_adding_a_child_to_its_current_parent_keeps_the_relationship` are rows of the edit table. `recursive_despawn_handles_deep_hierarchies_iteratively` became `despawn_takes_a_deep_hierarchy_without_recursing`.
- `despawn_callbacks.rs`: `despawning_a_leaf_detaches_it_and_preserves_siblings`, `despawning_the_last_leaf_removes_the_children_component`, `despawning_a_subtree_detaches_it_from_its_surviving_parent` are rows of the edit table.
- `deferred_callbacks.rs`: `callback_despawns_handle_deep_hierarchies_without_recursive_flushing` was an identical duplicate; `despawn_takes_a_deep_hierarchy_without_recursing` covers it.

Ordering
- tests/ecs/ordering.rs: all 18 order tests are rows of `a_schedule_runs_each_system_once_in_constraint_then_registration_order`. `a_cycle_created_by_set_expansion_panics` is a row of `a_cycle_in_the_constraints_panics_naming_its_systems`.
- system/schedule.rs, rows of the ordering table:
  - `without_a_filter_tags_change_nothing`, `an_include_filter_runs_only_tagged_systems`, `an_exclude_filter_drops_tagged_systems_and_wins_over_include`, `an_explicit_order_through_a_stand_in_still_holds`, `tagging_a_tuple_tags_every_system_in_it`
  - `a_self_referential_constraint_does_not_deadlock_compile`, `an_explicit_constraint_across_a_sync_point_compiles`
  - `chain_adds_one_edge_between_consecutive_systems`, `chaining_a_nested_group_orders_it_as_a_unit`, `after_and_before_produce_one_edge_each`: graph-edge internals, now asserted as run order.
- system/schedule.rs, other dispositions:
  - `multiple_systems_added`: trivia.
  - `a_constraint_does_not_register_its_target`: the "target not in the schedule" row.
  - `a_system_added_twice_is_two_nodes`: the "every copy of a twice added target" row.
  - `contradicting_explicit_constraints_panic` and `a_cycle_panic_names_both_systems`: the cycle table.
- executor/single_thread.rs: `after_dep_runs_before_main_at_runtime`, `before_main_runs_before_dep_at_runtime`, `nested_chain_runs_in_correct_order` are the data-disjoint rows ("constraints alone…", "before alone…").

Lifecycle and signals
- component_lifecycle.rs:
  - `insert_fires_on_add`, `remove_component_fires_on_remove_before_removal`, `despawn_fires_on_despawn_while_still_readable`, `callbacks_balance_across_mixed_operations`: rows of `add_and_remove_callbacks_fire_once_each_while_the_component_is_readable`.
  - `noop_remove_of_missing_component_is_harmless`, `remove_component_keeps_swapped_entity_intact`, `despawn_keeps_swapped_entity_intact`: rows of the storage table.
  - `on_replace_reads_the_old_value_and_skips_the_first_insert` and `on_replace_does_not_fire_for_a_component_added_alongside_an_existing_one`: `on_replace_reads_the_old_value_only_when_a_value_is_overwritten`.
- lifecycle_signals.rs:
  - Six operation tests are rows of `each_operation_runs_its_hook_then_its_signal_while_the_value_is_readable`.
  - The two whole-world refusal tests are rows of `a_lifecycle_listener_that_takes_the_whole_world_is_refused_by_name`.
- missing_resources.rs: four run_alone tests are rows of `a_system_whose_resource_or_event_is_missing_is_skipped`. The two executor tests are now `each_executor_skips_a_system_with_a_missing_resource_and_runs_the_rest`.
- validation_errors.rs:
  - `an_error_names_its_input_and_kind_and_displays_both_parts`: trivia (Display formatting). Naming is still asserted by `a_missing_resource_warns_on_every_skipped_run_naming_the_system_and_the_input`.
  - `apply_after_a_skipped_run_applies_nothing`: system/mod.rs `a_system_whose_parameter_declines_does_not_run_or_queue`.
- system_set.rs: three tests became `interned_sets_are_equal_only_for_the_same_type_and_value`.
- resource_get_or_insert.rs: three tests became one.
- signals.rs: `queued_entity_signals_are_deferred_and_observe_command_order` is merged into `queued_signals_are_deferred_and_observe_command_order`.

Maths, ids, executor, resources
- reachability.rs: all 8 tests are covered by the proptest `try_add_edge_keeps_the_graph_acyclic_and_reaches_its_transitive_closure`.
- entity/mod.rs: `id_strings_round_trip` is covered by the proptest `an_id_string_parses_back_to_its_entity`. `id_string_is_index_then_generation` and `malformed_ids_do_not_parse` became `an_id_string_is_index_v_generation_and_nothing_else_parses`.
- multi_thread.rs:
  - `a_panicking_system_is_raised_after_its_running_neighbours_finish…`, `a_lone_panicking_system_at_the_start…`, `a_lone_panicking_system_after_a_parallel_pair…`: rows of `a_panicking_system_is_raised_after_its_running_neighbours_and_its_dependants_never_run`.
  - `a_chain_of_send_systems_runs_on_the_calling_thread`, `an_exclusive_system_after_a_parallel_pair_runs_on_the_calling_thread`, `a_non_send_system_after_a_parallel_pair_runs_on_the_calling_thread`: rows of `work_that_must_stay_on_the_calling_thread_runs_there`.
- resource.rs: the four optional-resource tests became `an_optional_resource_is_none_while_absent_and_some_while_present` and `an_optional_resource_declares_the_access_of_a_plain_one`.
- query/filter.rs: two tests became `a_filter_conflicts_with_a_writer_only_when_it_reads_the_component`.
- input_set.rs: `an_input_set_of_another_marker_sees_none_of_these_members` became assertions in `an_input_set_declares_the_union_of_its_own_members_only`.

Scene and reflection
- component_registry.rs: `registered_component_is_deserialized_and_applied`, `full_type_path_also_resolves`, `re_registering_the_same_type_is_idempotent`, `unregistered_component_name_is_reported_not_fatal`, `stored_data_of_the_wrong_shape_spawns_the_default`: rows of `a_scene_component_is_found_by_its_short_or_full_name_and_spawned_from_its_stored_value`.
- reflect_component.rs: `a_resolved_entity_reference_is_its_id_string_in_json`, `an_unresolved_entity_reference_is_its_node_index_in_json`, `an_entity_reference_does_not_decode_from_a_malformed_id`: rows of `an_entity_reference_is_its_id_string_when_resolved_and_its_node_index_when_not`.
- scene_component.rs: `a_scene_component_spawns_from_a_stored_value_with_its_references_mapped`, `an_unset_reference_spawns_unset_without_a_warning`, `a_reference_past_the_scene_other_than_unset_warns` are rows of `a_stored_reference_spawns_as_its_node_and_one_past_the_scene_spawns_unset`. `a_reference_out_of_range_keeps_its_unresolved_default` is the same row as the past-the-scene case.
- component_reflection.rs: `lists_the_registered_components_an_entity_carries`, `unregistered_components_are_invisible`, `an_entity_with_no_registered_components_lists_nothing`: rows of `an_entity_lists_only_the_registered_components_it_carries`.

NOT VERIFIED: nothing. ECS only, no display needed.

CONCERNS:
- **Kept tests that read internals.** `write_skips_a_column_that_already_has_a_value` (required.rs) is kept because it is the only cover for the public `RequiredComponent::write` guard: breaking the guard fails no world-level test. `a_stand_in_has_no_access_and_no_sync_point` (schedule.rs) reads `compiled_data` internals; it is kept as the only cover that filtered-out systems add no dependencies.
- **Deleted tests with blind spots.** Breaking the swapped-row location update in `remove_component` failed none of the original swap tests; the storage table catches it only because it spawns a filler row afterwards. Separately, the "an entity holds a requirer" row of the external-requirement panic table cannot tell the requirer check from the plain type check, because a Requirer entity always holds Base too. That blind spot was in the original test as well.
- **Lockfile.** The Cargo.lock change is the first time proptest is locked in the workspace (task 1 declared it but no crate used it). It also re-resolves quick-error into two versions and adds borsh-derive.
- **#619 is partly done.** Tuple types in tests I rewrote are fixed. The `(Skips, Missing)` parameters in validation_errors.rs remain on purpose: tuple system inputs are the API that test exercises. Tuple types in untouched tests elsewhere are not swept.
- **Doc comments left in.** Existing `//!` header comments in component_lifecycle.rs, component_registry.rs and component_reflection.rs were left alone as AGENTS.md asks, though they now describe tables.

## Task 4: reflect, asset-format, foundation, scene, name, tasks, app, window, winit

STATUS: DONE WITH CONCERNS

COMMIT: (task 4 is 10 commits plus this report, oldest first)
- 268148f Turn the reflect codec, path, registry and apply families into tables
- a35a8ab Turn the content scan, content.toml and registry refusal families into tables
- 48b156a Use expect_err for a refused content scan
- 32dda68 Turn the foundation asset, derived data and maths families into tables and properties
- ea880ea Turn the scene startup and skeleton families into tables and merge the spawn round trips
- c02ff1f Replace the name interning family with a property and merge its encodings
- 3bb2633 Merge the task pool spawn and scope tests and table its unwind cases
- 94e8432 Turn the app role, extract change and registry families into tables
- b8d587d Turn the window input, gesture, clipboard and action families into tables and scenarios
- 676ab3b Turn the winit translation, settings and gamepad families into tables

GATE (CARGO_INCREMENTAL=0; `-p` concerto-reflect, concerto-reflect-macros, concerto-asset-format, concerto-foundation, concerto-scene, concerto-name, concerto-tasks, concerto-app, concerto-window, concerto-winit):
- `cargo fmt --all -- --check`: pass
- `cargo clippy --locked <those -p> --all-targets -- -A clippy::type_complexity -A clippy::too_many_arguments -D warnings`: pass
- `cargo test --locked <those -p>`: pass (no failures; the one ignored doctest in foundation was ignored before)
- `cargo metadata --format-version 1`: run after each `Cargo.toml` change; the lockfile gains `proptest` under concerto-foundation and concerto-name.

COUNTS (`#[test]`, including those inside `proptest!`, at 211d380 and at 676ab3b):
- reflect (with reflect-macros): 88 -> 34
- asset-format: 27 -> 7
- foundation: 170 -> 80 (8 of them proptest properties)
- scene: 25 -> 12
- name: 9 -> 4 (1 property)
- tasks: 9 -> 4
- app: 46 -> 29
- window: 56 -> 28
- winit: 34 -> 11
- total: 464 -> 209

CHANGED:
- crates/foundation/Cargo.toml, crates/name/Cargo.toml: `proptest.workspace = true` as a dev-dependency. Cargo.lock: proptest for both (the name half landed in ea880ea).
- crates/foundation/src/assets/asset_server.rs: Work 3 rows `:392 is_loaded` and `:397 resolve_by_id` removed; the test module is rewritten around a `Fixture`/`Saved` pair (no tuple returns), with life, cancel and on_load tables.
- crates/foundation/src/assets/{asset_store,handle}.rs, src/math/{spring,spring_damper}.rs, src/geometry/delauney.rs, src/lib.rs: tables, properties, merges; lib.rs loses its assertion-free test.
- crates/foundation/tests/foundation/*.rs: tables and properties per the DELETED list; check_encoding_bless.rs and the encoding fixtures are untouched.
- crates/reflect/tests/reflect/*.rs, crates/reflect/macros/src/lib.rs: tables; the macro crate's test module is removed.
- crates/asset-format/tests/asset-format/scan.rs: scan, content.toml and refusal tables.
- crates/scene/tests/scene/*.rs: tables and merges; the tuple-returning `comparable` is a struct.
- crates/name/tests/name/name.rs, crates/tasks/src/task_pool.rs: property, merges, unwind table.
- crates/app/src/{lib,extractor,inspected_asset}.rs, src/plugins/mod.rs, tests/app/{event_lifetime,nested_app}.rs: tables and merges; the nested-app channel sends a struct, not a tuple.
- crates/window/src/{events,window,plugin,clipboard}.rs, src/clipboard/native.rs, src/input/gamepad.rs, tests/window/actions.rs: tables and scenarios.
- crates/winit/src/{translate,apply,gamepad}.rs: tables.
- No production behaviour changed. No encoding fixture was blessed or rewritten.

BREAK-AND-REVERT: listed per crate below under BREAKS. Each named edit was made, the table or property failed, and the edit was reverted before committing.

DELETED and BREAKS, by crate:

reflect (88 -> 34), commit 268148f
DELETED:
- macros/src/lib.rs `a_crate_depending_on_concerto_reaches_reflect_through_it`, `a_crate_depending_on_concerto_reflect_uses_it_directly`: private non-maths function; every path is compiled by the workspace build (tech-demo derives Reflect through `concerto` only, engine crates through `concerto-reflect`, the editor depends on both).
- value.rs `a_field_the_tree_lacks_keeps_its_default`, `a_stored_field_the_type_lacks_is_ignored_with_a_warning_naming_its_path`, `a_node_of_the_wrong_kind_leaves_the_field_unchanged_with_a_warning`, `a_leaf_that_does_not_decode_leaves_the_field_unchanged_with_a_warning`, `an_unknown_variant_leaves_the_enum_unchanged_with_a_warning`, `a_list_grows_and_shrinks_to_the_stored_length`, `warnings_name_nested_paths_through_lists_and_variants`: rows of `a_stored_tree_writes_what_matches_and_warns_at_the_path_of_what_does_not`.
- kinds.rs `a_vec_is_a_list_that_grows_with_defaults_and_shrinks`, `removing_from_a_vec_takes_that_element_out`, `removing_out_of_range_from_a_vec_changes_nothing`: merged into `a_vec_is_a_list_that_grows_with_defaults_shrinks_and_removes_in_range`. `a_generic_struct_reflects_its_parameter`: merged into `named_tuple_unit_and_generic_structs_are_structs_with_named_fields`.
- derive.rs `access_resolves_nested_leaf_composite_and_root`, `unknown_paths_and_paths_past_opaque_values_do_not_call_back`, `access_reaches_list_elements_and_what_they_hold`, `an_index_out_of_range_or_off_a_list_and_a_field_on_a_list_are_not_found`: rows of `a_path_reaches_exactly_the_property_it_names_for_reading_and_writing`. `mutable_access_can_edit_nested_fields_composites_and_root`, `mutable_access_reaches_list_elements_and_what_they_hold`: `a_write_through_a_path_lands_in_the_property_it_names` plus the mutable half of the access table. `paths_preserve_names_depth_and_segments`, `a_path_mixes_field_names_and_indices`, `a_paths_name_is_its_last_field`, `a_segment_equals_the_text_of_its_field_name_only`, `a_path_displays_its_segments_joined_by_dots`: `a_path_is_named_by_its_last_field_and_displays_its_segments_joined_by_dots` (depth/get_depth getters: trivia). `visit_reports_each_list_element_in_order`, `visit_mut_reports_each_list_element_in_order`: `visiting_a_list_reports_each_element_in_order_and_never_a_field`. `an_options_value_is_a_field_named_zero`: the `fog.0.weight` rows of the access table and the Option assertion in kinds `visiting_sees_..._and_an_options_value_as_zero`. `downcast_reads_the_concrete_type_and_refuses_another`: trivia (delegates to Any); downcast_ref/downcast_mut are exercised by the access and visit tests. `a_skipped_field_is_not_a_field`, `a_skipped_field_keeps_its_value_when_a_stored_value_is_applied`: merged into `a_skipped_field_is_not_a_field_and_keeps_its_value_when_a_stored_value_is_applied`.
- registry.rs `a_type_declaring_nothing_has_no_type_data`: merged into `a_registration_carries_its_names_and_only_the_type_data_its_derive_declared`. `registering_twice_changes_nothing`, `registering_a_type_registers_the_types_its_fields_hold`, `a_type_that_holds_itself_registers_once`, `an_opaque_type_registers_none_of_its_fields`: rows of `registering_a_type_adds_it_and_the_types_its_fields_hold_once` (now exact sets, not presence). `a_name_resolves_by_full_path_first_then_by_short_name` and `two_types_sharing_a_short_name_resolve_to_the_last_registered_with_a_warning`: merged into `a_name_resolves_by_full_path_then_by_the_last_registered_short_name_with_a_warning`. `a_unit_only_enum_registers_its_variant_names_in_order`, `an_enum_with_tuple_and_named_variants_registers_its_variant_names_in_order`, `an_option_registers_none_and_some_and_the_option_marker`, `a_vec_field_registers_as_a_list_with_nothing_declared`, `an_option_is_not_a_list`, `a_struct_and_an_opaque_enum_register_no_variant_names`: rows of `an_enum_registers_its_variant_names_in_order_an_option_its_marker_and_a_vec_its_list`.
- json.rs `bool_is_a_json_boolean`, `integers_are_json_numbers_at_their_full_range`, `floats_are_json_numbers_in_their_shortest_form`, `a_float_decodes_from_an_integer`, `a_leaf_written_from_json_is_the_codecs_value_encoded`, `non_finite_floats_are_strings`, `string_is_a_json_string` (encode half), `vectors_and_quaternions_are_arrays_of_their_components`, `a_color_is_an_array_of_its_linear_components`, `a_color_in_another_space_is_written_as_its_linear_components`: rows and trailing assertions of `each_leaf_with_a_codec_is_written_as_its_json_and_built_back_from_it`. `an_integer_out_of_range_or_not_an_integer_does_not_decode`, `a_float_does_not_decode_from_a_finite_string_or_out_of_range`, `string_is_a_json_string` (refusal), `a_vector_of_the_wrong_length_or_with_a_non_number_does_not_decode`, `a_color_of_the_wrong_length_or_with_a_non_number_does_not_decode`, `a_leaf_that_does_not_decode_names_its_path_and_shape`, `a_struct_that_is_not_an_object_names_its_fields`, `an_unknown_field_is_named_with_the_fields_there_are`, `an_enum_names_the_variant_shape_it_expects`, `a_list_that_is_not_an_array_says_so`, `an_opaque_leaf_cannot_be_written`, `a_type_with_no_codec_and_no_registered_default_cannot_be_built`: rows of `json_that_does_not_fit_the_type_is_refused_naming_the_path_and_the_shape_it_expects` (every row now pins the exact message). `fields_left_out_of_the_json_are_not_written`, `a_variant_switched_by_name_alone_starts_from_its_defaults`, `a_list_written_from_json_takes_the_json_length`: rows of `a_partial_write_changes_only_what_the_json_names`.
BREAKS: apply skipping the no-such-field warning; PropertyPath::name stopping at an index; path access reading index-1; register skipping register_dependencies; Option variants reversed; non-finite floats upper-cased; vector arrays accepting extra components; list refusal text changed; apply not truncating lists (partial write table).

asset-format (27 -> 7), commits a35a8ab and 48b156a (the clippy fix: `expect_err` for a refused scan)
DELETED:
- `scanning_honours_the_extension_and_a_non_canonical_base`, `scanning_an_absent_directory_lists_nothing`, `a_duplicate_id_is_an_error_naming_both_addresses`, `an_unreadable_header_is_an_error_naming_its_path`, `a_symlinked_content_directory_keeps_content_addresses`, `a_symlink_cycle_lists_each_entry_once_and_terminates`, `a_directory_reached_through_two_paths_is_listed_once_under_the_first_name`, `a_directory_outside_the_base_keeps_the_walked_path_as_its_address`, `a_base_with_parent_after_a_symlink_is_normalised_lexically`, `a_dangling_directory_symlink_is_skipped`, `two_symlinks_to_one_directory_list_it_once_under_the_first_name`: rows of `a_scan_lists_each_file_of_the_extension_once_under_the_first_path_it_walks` (symlink rows are `#[cfg(unix)]`, as the tests were).
- `a_scan_reads_the_kind_version_and_registry_text_has_none`: the scan half by `scanning_a_tree_lists_every_file_of_the_extension_relative_to_the_base` (asserts `kind_version: Some(1)`); the registry half by `the_registry_text_round_trips_without_the_kind_version` (an entry with `Some(3)` parses back with `None`).
- `parsing_rejects_the_legacy_address_only_form`, `parsing_rejects_two_ids_at_one_address`: rows of `parsing_refuses_the_legacy_address_only_form_and_two_ids_at_one_address`.
- `content_config_defaults_when_content_toml_is_absent`, `content_config_fills_a_missing_field_from_the_default`, `content_config_reads_the_default_scene`, `content_config_has_no_default_scene_unless_one_is_set`, `content_config_rejects_a_root_other_than_content`, `content_config_rejects_a_bad_extension`, `content_config_accepts_a_custom_extension_with_the_default_root`, `an_unreadable_content_toml_is_an_error`: rows and the trailing assertion of `content_toml_fills_its_defaults_and_refuses_a_root_or_extension_it_cannot_use`.
- `validate_rejects_a_hand_built_bad_config`: `load_or_default` validates through `validate`, so the refusal rows of the content.toml table cover it.
BREAKS: scan ignoring the visited set; validate accepting any root but "data"; registry keying addresses by id (no duplicate-address refusal).

foundation (170 -> 80), commit 32dda68
Work 3: `asset_server.rs:392 is_loaded` removed (tests read the public `is_loading_or_loaded`, which equals it when nothing is pending); `:397 resolve_by_id` removed (tests read the public `registry()`).
DELETED:
src/lib.rs
- `test_add_transform`: trivia (no assertion).
src/math/spring.rs
- `framerate_independent`, `quat_is_framerate_independent`: proptest `one_step_lands_where_many_smaller_steps_do` (f32 and Quat).
- `vec3_converges`: merged into `a_float_and_a_vector_converge_to_their_target`.
- `integral_of_a_critically_damped_spring_from_rest`: `integral_is_the_area_under_the_value_in_every_damping_regime` (its 1.0 damping row).
src/math/spring_damper.rs
- `halflife_and_damping_round_trip`: proptest `halflife_and_damping_convert_back_and_forth`.
- `decay_is_framerate_independent`: proptest `one_decay_step_lands_where_many_smaller_steps_do`.
- `a_rotation_of_a_milliradian_keeps_its_angle` and the round-trip line of `scaled_angle_axis_round_trips`: proptest `a_scaled_angle_axis_survives_a_quaternion_round_trip` (magnitudes 1e-5 to 2.8, tolerance min(1e-3·|v|, 1e-5)); the rest of that test is `a_scaled_angle_axis_is_the_rotation_angle_along_its_axis`.
src/geometry/delauney.rs
- `single_triangle`, `square_gives_two_triangles`, `five_points_with_interior`, `fewer_than_three_points_is_empty`: rows of `a_triangulation_has_two_n_minus_hull_minus_two_triangles`.
- `locate_center_of_triangle`, `locate_at_vertex`, `locate_or_nearest_exact_when_inside`, `barycentric_coords_sum_to_one`: proptest `a_point_inside_locates_to_weights_that_rebuild_it` (vertices are sampled explicitly).
- `locate_outside_returns_none`, `locate_or_nearest_projects_to_boundary`: proptest `a_point_outside_is_not_located_and_its_nearest_is_on_the_hull`.
src/assets/handle.rs
- `a_handle_is_its_asset_id_as_32_hex_digits_in_json`, `a_handle_does_not_decode_from_anything_but_an_asset_id`, `a_handle_written_back_from_its_json_names_the_same_asset`, `a_handle_written_from_anything_but_an_asset_id_says_what_it_expected`: merged into `a_handle_is_its_asset_id_as_32_hex_digits_in_json_and_nothing_else_decodes`.
src/assets/asset_store.rs
- `a_handle_requested_before_the_drop_is_processed_keeps_the_asset_loaded`, `a_dropped_asset_with_no_returned_handle_unloads`, `dropping_the_returned_handle_unloads_the_asset_on_the_next_drain`, `a_duplicate_drop_event_for_an_unloaded_id_is_ignored`: rows of `an_asset_unloads_once_its_last_handle_drops_and_a_drain_sees_it`.
src/assets/asset_server.rs
- `resolve_by_id_finds_a_registered_asset`, `resolve_by_id_returns_none_for_an_unregistered_id`: `the_server_resolves_through_the_registry_resource_once_it_changes` (None, then the address) and `load_reads_a_registered_content_asset_through_the_registry`.
- `a_handle_registered_through_its_component_describes_itself_without_naming_its_asset`, `a_described_handle_loads_only_through_a_server_that_registered_its_asset`: merged into `a_handles_type_data_describes_it_and_sets_it_only_through_a_server_that_registered_its_asset`.
- `a_handle_written_through_live_leaves_leaves_loads_through_the_server`, `live_leaves_keep_entity_references_live_and_handles_weak_without_a_server`: merged into `live_leaves_load_a_handle_through_the_server_or_keep_it_weak_without_one_and_keep_entity_references_live`.
- `load_fails_for_an_unregistered_id`, `load_fails_without_reading_when_the_registry_names_another_kind`, `a_load_is_in_flight_until_its_asset_reaches_the_world`, `a_failed_load_is_in_flight_until_its_failure_is_handled`, `an_added_asset_is_in_flight_until_it_reaches_the_world`, `an_id_is_loading_or_loaded_until_its_load_fails`, `an_id_stays_loading_or_loaded_once_its_asset_lands`, `add_runs_on_load_before_the_asset_reaches_the_world`, `add_whose_on_load_fails_sends_no_loaded_event_and_the_handle_never_resolves`: rows of `a_request_is_in_flight_until_its_asset_or_failure_reaches_the_world` (the fixture's on_load now refuses u32::MAX instead of a separate FailingAsset).
- `a_load_that_finished_but_was_not_yet_handled_is_not_inserted_once_cancelled`, `cancelled_loads_are_no_longer_in_flight`, `cancelling_keeps_an_added_asset`: rows of `cancelling_drops_every_pending_load_and_keeps_added_assets`.
- `on_load_reads_type_data_from_the_registry_given_to_the_server`, `on_load_sees_nothing_in_a_server_without_a_shared_registry`, `a_type_registered_after_the_registry_was_shared_is_visible_to_a_later_load`: rows of `on_load_reads_type_data_from_the_registry_shared_with_the_server_when_it_loads`.
- `a_server_knows_only_the_asset_types_registered_with_it`: trivia (getter); `is_registered` is still asserted in the type-data test, whose refusal depends on it.
tests/foundation/asset_id.rs
- `from_path_is_deterministic`, `from_path_differs_for_different_inputs`: merged into `from_path_is_the_same_for_the_same_path_and_differs_for_another`.
tests/foundation/asset_handle.rs
- `weak_handle_serializes_to_its_id`, `deserialized_handle_is_weak_and_id_matches`: merged into `a_handle_encodes_as_its_bare_id_and_decodes_weak` (now also asserts the bytes equal the bare id's).
tests/foundation/check_encoding.rs
- `matching_bytes_pass`, `a_missing_fixture_panics_with_the_bless_hint`, `differing_bytes_panic_asking_for_a_version_bump`: rows of `check_encoding_passes_only_on_identical_bytes_and_says_how_to_fix_a_mismatch`.
tests/foundation/asset_registry.rs
- `insert_get_remove_round_trip_in_memory`, `id_for_address_is_the_inverse_of_get`, `inserting_an_existing_id_moves_it_and_frees_its_old_address`, `inserting_a_new_id_at_a_held_address_replaces_the_entry_there`: proptest `each_id_and_each_address_name_one_entry_listed_by_folder_then_name` (model-checked insert/remove sequences).
- `read_content_asset_header_does_not_read_the_payload`: the header-from-file reads of the `each_reader_reads_versions_4_to_6...` table (the test could not observe a prefix read anyway).
- `parse_rejects_two_ids_sharing_one_address`, `parse_rejects_the_old_format_and_says_how_to_rebuild`: `AssetRegistry::parse` is `parse_registry`; asset-format `parsing_refuses_the_legacy_address_only_form_and_two_ids_at_one_address`.
- `from_content_tree_repoints_a_moved_file_at_its_new_address`: merged into `from_content_tree_indexes_every_gasset_by_its_header_id_and_follows_a_moved_file`.
- `from_content_tree_rejects_two_files_sharing_an_id`, `from_content_tree_of_an_absent_tree_is_empty`: `from_content_tree` is `scan_content`; rows of asset-format `a_scan_lists_each_file_of_the_extension_once_under_the_first_path_it_walks`.
- `save_content_asset_into_mints_an_id_not_derived_from_the_address`, `saving_over_an_existing_asset_reuses_its_id`: `a_save_writes_a_readable_asset_under_a_minted_id_and_a_re_save_keeps_it`.
tests/foundation/save_content_asset.rs
- `writes_a_readable_content_asset_under_the_project_root`, `creates_missing_parent_directories`, `saving_into_a_registry_returns_the_id_and_records_it`, `a_save_writes_the_type_version`: merged into `a_save_writes_a_readable_asset_under_a_minted_id_and_a_re_save_keeps_it`.
- `a_resave_fails_without_a_file`, `a_resave_fails_over_another_kind_and_leaves_the_file`: merged into `a_resave_refuses_a_missing_file_or_another_kind_and_changes_nothing`.
tests/foundation/decode_content_asset.rs
- `the_derive_defaults_to_version_one_and_reads_the_attribute`, `an_equal_version_decodes_the_payload`, `an_older_version_goes_through_migrate`, `an_older_version_without_a_migration_is_rejected`, `a_newer_version_is_rejected`, `a_wrong_kind_is_rejected`: rows of `a_payload_decodes_at_its_version_migrates_from_an_older_one_and_is_refused_otherwise` (the derive's default and attribute versions are the first two rows).
tests/foundation/content_first_loading.rs
- `reads_a_content_asset_at_its_literal_address`, `a_kind_mismatch_is_an_error`, `a_missing_content_asset_is_an_error`, `a_directory_at_the_address_is_an_error`: rows of `a_content_asset_loads_from_its_literal_address_and_a_failure_names_the_address`.
tests/foundation/content_asset_format.rs
- `round_trips_header_and_payload_verbatim`, `empty_payload_is_valid`, `a_cooked_file_round_trips_header_payload_and_blob`, `read_content_asset_leaves_the_derived_blob_out_of_the_payload`, `a_file_without_a_section_has_no_derived_blob`: proptest `a_written_asset_reads_back_its_header_payload_and_derived_blob`.
- `rejects_a_buffer_without_the_magic`, `rejects_a_truncated_header`, `a_version_5_header_reads_with_no_derived_section`, `rejects_a_section_longer_than_the_file`, `read_content_asset_header_reads_a_cooked_header`, `a_version_4_header_reads_as_kind_version_1_through_both_readers`, `rejects_versions_3_and_7_through_both_readers`: rows of `each_reader_reads_versions_4_to_6_and_refuses_anything_else_saying_why`.
- `writes_header_version_6_after_the_length`: `the_cooked_framing_matches_its_fixture` (the committed bytes pin the version field).
tests/foundation/derived_data.rs
- `a_load_derives_applies_the_blob_and_writes_one_cache_file_at_its_path`, `a_second_server_over_the_same_cache_loads_without_deriving`, `changing_an_input_changes_the_key_and_derives_again`, `a_raised_derived_version_misses_the_old_entry`, `a_cached_blob_that_apply_refuses_is_derived_again_and_overwritten`: steps of `the_cache_keys_a_blob_by_version_and_input_bytes_and_replaces_one_apply_refuses`.
- `a_cache_whose_root_is_a_file_still_loads_with_the_blob_in_memory`, `with_no_cache_every_load_derives`: rows of `without_a_writable_cache_every_load_derives_with_the_blob_in_memory`.
- `a_cooked_blob_loads_without_its_inputs_and_without_deriving`, `a_cooked_blob_of_another_derived_version_is_derived_again`, `a_cooked_blob_that_apply_refuses_is_derived_again`: rows of `a_cooked_blob_of_the_current_version_loads_without_inputs_and_any_other_is_derived_again`.
- `a_derived_section_on_an_asset_that_is_not_derived_is_ignored`, `a_cooked_section_on_an_instance_that_does_not_derive_is_not_applied`: merged into `a_derived_section_on_an_asset_or_instance_that_does_not_derive_is_ignored`.
- `a_missing_input_fails_the_load_naming_it`, `a_derived_input_fails_the_load_naming_it`, `a_derive_error_fails_the_load`, `a_missing_further_input_fails_the_load_naming_it`, `a_further_input_that_derives_is_refused`: rows of `a_load_fails_on_a_missing_or_derived_input_or_a_derive_error_naming_the_input`.
- `a_cook_skips_entries_that_are_not_derived`, `only_derived_kinds_can_be_cooked`: merged into `a_cook_writes_each_derived_entry_with_its_payload_key_and_blob_and_nothing_else`.
- `a_cook_uses_a_cached_blob`, `a_cook_derives_again_over_a_cached_blob_that_apply_refuses`: rows of `a_cook_uses_a_cached_blob_unless_apply_refuses_it`.
- `an_instance_that_does_not_derive_loads_without_reading_inputs_or_touching_the_cache`, `an_instance_that_derives_loads_with_its_blob`: merged into `an_instance_derives_only_when_it_says_so_and_one_that_does_not_reads_no_inputs`.
- `changing_a_further_input_changes_the_key_and_derives_again`: merged into `further_inputs_are_read_and_seen_by_the_key_and_the_derive`.
- `a_cook_reads_further_inputs`, `a_missing_further_input_stops_the_cook_naming_it`: merged into `a_cook_reads_further_inputs_and_stops_naming_one_that_is_missing`.
Kept: transform/systems.rs (deep hierarchy, changed marks), frame_stats (3), hash (FNV constants), assets/mod.rs content_root_for_exe and directory_of (private but the only cover of how a shipped or web game finds its content), the scene-handle capture test, uuid sharing, serialized handle, gated cancel, self-loading add, the encoding fixture tests and check_encoding_bless.rs.
BREAKS: Spring::update with a dt-dependent coefficient (step property); halflife_to_damping constant 4→3; to_scaled_angle_axis via acos(w) (small-angle precision); decay exponent dt·(1+dt); triangulate needing 4 points (count table); locate swapping λb/λc (inside property); locate_or_nearest picking the farthest triangle (outside property); is_loading ignoring the event channel (life table); cancel_pending keeping cancelled loads' events (cancel table); process_handle_drop ignoring live handles (lifetime table); set_type_registry discarding the registry (on_load table); decode_content_payload skipping the kind check (decode and load tables); format versions 3 and 7 failing with an empty message (reader table); write_content_asset dropping the last payload byte (round-trip property); check_encoding's mismatch text changed (check table); registry insert not freeing a held address (registry property); DerivedDataCache::put panicking on a write error (uncached table); apply_cooked ignoring the derived version (cooked table); the derived-input refusal disabled (failure table); the cook ignoring cached blobs (cook-cached table).

scene (25 -> 12), commit ea880ea (it also carries the one-line Cargo.lock change for name's proptest dev-dependency)
DELETED:
- startup_scene.rs `a_game_app_spawns_its_declared_startup_scene`, `an_edit_app_does_not_spawn_the_startup_scene`, `without_a_declaration_nothing_is_spawned`, `a_preview_app_does_not_spawn_the_startup_scene`: rows of `only_a_game_app_spawns_the_startup_scene_it_declares`.
- scene_serialization.rs `a_new_scene_is_empty`: trivia (constructor).
- scene_skeleton.rs `skeleton_component_resolves_scene_references_in_place`, `rootless_skeleton_component_supports_primitive_bindings`, `invalid_references_remain_unresolved_without_shifting_bone_indices`, `a_scene_that_still_stores_bone_ids_loads_with_the_field_ignored`: rows of `a_spawned_skeleton_resolves_its_references_in_place_and_leaves_bad_ones_unset`.
- capture_scene.rs `a_spawned_scene_captures_back_to_what_was_spawned`, `spawning_through_the_world_matches_the_queued_spawner`: merged into `a_scene_spawned_by_command_or_in_the_world_captures_back_to_what_was_spawned`. `a_subtree_referring_outside_itself_cannot_be_captured`: merged into `a_subtree_capture_starts_at_the_entity_remaps_inner_references_and_refuses_outer_ones`.
- spawn_scene.rs `reusable_spawn_returns_source_mapping_and_preserves_registered_components`: the merged spawn/capture test (node and root entities, components, and the hierarchy under the parent, read back through capture). `spawned_nodes_carry_their_authored_name`: the same test (capture reads every node's Name, including the component-less "sibling"). `scene_round_trips_through_bincode_with_component_payloads`: scene_serialization `round_trips_and_reports_references` and the scene encoding fixture. `unregistered_component_is_skipped_without_failing_the_node`: ecs `a_scene_component_is_found_by_its_short_or_full_name_and_spawned_from_its_stored_value` (its unregistered-name row).
- The tuple-returning `comparable` helper became a `Comparable` struct; the tuple destructure of the mesh query map in `spawner_expands_nodes_and_upgrades_weak_handles` became direct assertions.
BREAKS: the startup spawn added `at_edit_time` (startup table); scene reference spawn wrapping the node index modulo the node count (skeleton table).

name (9 -> 4), commit c02ff1f
DELETED:
- `the_same_text_interns_to_equal_names`, `different_texts_intern_to_different_names`: proptest `two_names_are_equal_exactly_when_their_texts_are`.
- `a_name_reads_back_its_text`: `as_str` and `From<String>` are in the property; the `Display`/`Debug`/`PartialEq<&str>` assertions are trivia.
- `the_default_name_is_the_empty_text`: trivia (named in the spec).
- `a_name_encodes_with_bincode_as_the_string_of_its_text`, `a_name_round_trips_through_its_reflected_value`, `a_name_writes_and_reads_json_as_a_string`: merged into `a_name_is_stored_as_its_text_in_bincode_reflection_and_json`.
BREAKS: Name::new looking up the lower-cased text (property).

tasks (9 -> 4), commit 3bb2633
DELETED:
- `spawn_returns_result`, `spawn_multiple_tasks_concurrently`: merged into `spawned_tasks_all_run_and_return_their_results`.
- `scope_collects_results`, `scope_with_no_tasks_returns_empty`: merged into `a_scope_returns_the_result_of_every_task_it_spawned`. `scope_shared_state_mutation`: the same test (every spawned task's result comes back, so every task ran).
- `drop_joins_threads`: trivia (no assertion; every test drops its pool).
- `a_scope_whose_task_panics_unwinds_only_after_its_other_tasks_finish`, `a_scope_whose_closure_panics_unwinds_only_after_its_tasks_finish`: rows of `a_scope_that_panics_unwinds_with_the_first_panic_only_after_its_other_tasks_finish`.
BREAKS: scope preferring the task panic over the closure panic (unwind table).

app (46 -> 29), commit 94e8432
DELETED:
src/lib.rs
- `an_app_for_editing_reports_its_role`, `an_app_for_previewing_is_also_editing`, `only_marked_systems_run_in_an_edit_app`, `preview_time_systems_run_in_game_and_preview_apps_only`, `a_preview_app_runs_edit_time_systems_and_not_unmarked_ones`, `only_marked_listeners_are_added_to_an_edit_app`, `preview_time_listeners_are_added_to_game_and_preview_apps_only`, `a_preview_app_adds_edit_time_listeners_and_not_unmarked_ones`: rows of `edit_and_preview_marks_decide_which_systems_and_listeners_each_kind_of_app_runs` (each row checks the app's role and runs all three marks through a system and a listener).
- `a_registered_plugin_is_reported_as_added`, `a_plugin_registered_by_another_plugin_is_reported_as_added`: merged into `a_plugin_registered_directly_or_by_another_plugin_is_reported_as_added`.
src/extractor.rs
- `an_extract_system_sees_a_main_world_system_write_once`, `an_extract_system_sees_a_direct_main_world_write_between_extracts`, `an_extract_systems_first_run_sees_everything`, `a_skipped_extract_keeps_the_change`: rows of `an_extract_system_sees_each_main_world_change_once_even_across_a_skipped_run`.
src/inspected_asset.rs
- `registering_an_inspected_asset_records_each_kind_once`, `registering_an_inspected_asset_registers_the_asset_and_its_type`, `an_asset_registered_before_is_still_recorded`: merged into `registering_an_inspected_asset_records_each_kind_once_and_registers_the_asset_and_its_type` (Tag is registered as an asset first).
- `a_record_refuses_to_save_another_type`: merged into `a_record_creates_decodes_and_saves_its_asset_and_refuses_another_type`.
src/plugins/mod.rs
- `installing_content_settings_keeps_derived_data_under_the_project_root`, `installing_content_settings_without_a_project_root_sets_no_cache`: rows of `content_settings_keep_derived_data_under_the_project_root_or_nowhere_without_one`.
- `the_published_registry_becomes_a_resource_once_plugins_finish`, `a_registry_the_host_supplied_is_kept_when_plugins_finish`: rows of `the_registry_resource_once_plugins_finish_is_the_one_supplied_or_else_the_published_one`.
tests/app/event_lifetime.rs
- `a_reader_that_runs_every_frame_misses_nothing_whatever_the_registration_order`: merged into `event_lifetime_does_not_depend_on_when_the_event_was_registered` (four cases: two orders, two readers).
- nested_app.rs: the `(outer, inner)` tuple sent over the channel became a `Stepped` struct; the test is kept.
BREAKS: at_preview_time tagging EditTime (role table); Extracted not committing its ticks (extract table); AssetManagerPlugin::finish replacing a supplied registry (registry rows); install_content_registry defaulting the project root to "." (cache rows); the event channel update moved from First to Update (event lifetime rows).

window (56 -> 28), commit b8d587d
DELETED:
src/clipboard/native.rs
- `text_on_the_clipboard_is_read_as_text`, `an_empty_or_non_text_clipboard_reads_as_no_text`, `access_errors_read_as_unavailable`: rows of `a_clipboard_read_is_text_no_text_or_unavailable_with_the_reason` (private, but the only cover of how OS clipboard errors reach a paste).
src/clipboard.rs
- `a_write_is_pasted_back_from_the_in_process_clipboard`, `a_failed_read_answers_with_the_in_process_copy`, `a_clipboard_without_text_answers_empty`, `the_backends_text_wins_over_the_in_process_copy`: rows of `a_paste_answers_once_with_the_backends_text_or_else_the_in_process_copy`.
src/plugin.rs
- `a_point_inside_a_zone_returns_its_gesture`, `a_point_outside_every_zone_returns_none`, `the_first_of_two_overlapping_zones_decides`, `a_zone_without_a_gesture_blocks_the_zone_under_it`: rows of `a_point_takes_the_gesture_of_the_first_zone_holding_it`.
src/window.rs
- `an_input_event_reports_no_fact`: merged into `fact_events_update_the_facts_and_input_events_report_none`.
src/input/gamepad.rs
- `a_stick_reads_zero_inside_the_dead_zone_and_one_past_the_live_zone`, `a_stick_between_the_zones_rescales_its_length_and_keeps_its_direction`: rows of `a_stick_reads_zero_in_the_dead_zone_one_past_the_live_zone_and_rescales_its_length_between`.
src/events.rs
- `a_pushed_press_reads_as_just_pressed_through_late_update_then_as_held`, `a_release_event_releases_the_key`: merged into `a_press_reads_as_just_pressed_through_late_update_then_as_held_until_released`.
- `losing_focus_releases_every_held_key_and_button`, `gaining_focus_leaves_input_alone`: merged into `losing_focus_releases_every_held_key_and_button_and_gaining_it_changes_nothing`.
- `a_closed_gate_drops_keys_and_buttons_but_keeps_the_cursor`, `closing_the_gate_releases_held_input_on_the_next_fold`, `pad_events_fold_with_the_gate_closed`, `closing_the_gate_keeps_a_held_sticks_value`, `reopening_the_gate_folds_input_again`: steps of `a_closed_gate_releases_and_drops_keys_and_buttons_but_keeps_the_cursor_and_pads`.
- `connecting_and_disconnecting_write_one_connection_event_each`, `events_for_a_pad_that_is_not_connected_are_ignored`: merged into `a_pad_connects_and_disconnects_once_and_events_for_a_pad_not_connected_are_ignored`.
tests/window/actions.rs
- `a_global_binding_fires_with_no_context_pushed`, `modifiers_must_match_exactly`, `the_innermost_context_wins`, `a_context_binding_is_inert_while_its_context_is_not_active`, `a_context_binding_beats_a_global_one`, `typing_swallows_unmodified_keys_but_not_shortcuts`, `typing_does_not_swallow_navigation_keys`, `rebinding_an_action_replaces_the_old_shortcut`, `pushing_a_context_twice_still_pops_clean`: rows of `a_shortcut_resolves_to_the_innermost_active_binding_and_typing_swallows_only_text`.
- `keys_across_bindings_merge_into_one_normalized_direction`, `the_longer_of_the_keys_and_a_stick_wins`, `a_two_dimensional_action_is_zero_while_its_context_is_inactive`, `text_like_keys_read_zero_while_capturing_text`: rows of `a_two_dimensional_action_reads_the_longer_of_its_merged_keys_and_its_sticks`.
Kept: input/mod.rs (4 state-machine tests), the trigger, pad button, release_all and release_keys_and_buttons gamepad tests, cursor/motion and resize fold tests, text_read, paste order, the pad-button, rebind/unbind, held-with-modifiers and second-device action tests.
BREAKS: gesture_at giving an empty zone the Move gesture (gesture table); arboard_read treating ConversionFailure as unavailable (read table); NoText pasting the in-process copy (paste table); is_text_like ignoring modifiers (resolve table); the default live zone at 1.0 (stick table); axis2d letting a tied stick win (direction table).

winit (34 -> 11), commit 676ab3b
DELETED (the translation functions are private, but they are the only cover of how OS events become engine events):
src/translate.rs
- `a_press_carries_its_text`, `a_release_carries_no_text`, `control_characters_are_dropped_from_text`: rows of `a_key_press_carries_its_text_without_control_characters_and_a_release_carries_none`.
- `window_facts_translate`, `pointer_events_translate`, `a_pen_moves_the_cursor_as_the_mouse_does`, `a_touch_does_not_move_the_cursor`, `a_mouse_or_pen_press_is_a_mouse_button`, `a_touch_or_unknown_press_is_dropped`, `both_wheel_units_translate`, `events_with_no_engine_meaning_are_dropped`, `a_redraw_is_asked_for_again_while_there_is_no_app`, `the_close_button_exits_while_there_is_no_app`, `input_and_window_facts_are_dropped_while_there_is_no_app`: rows of `a_winit_event_becomes_its_engine_event_or_nothing_and_while_waiting_only_redraws_and_closes` (each row checks `translate` and `while_waiting`).
src/apply.rs
- `nothing_applied_yet_applies_every_setting`, `unchanged_settings_need_no_action`, `only_the_changed_settings_are_applied`: rows of `only_the_settings_not_yet_applied_become_actions`.
- `an_app_without_a_window_has_no_requests`: merged into `requests_are_read_once_and_an_app_without_a_window_has_none`.
- `an_app_without_a_window_needs_no_size_correction`, `a_window_of_the_actual_size_needs_no_correction`, `a_window_of_another_size_is_told_the_actual_size`: merged into `only_a_window_of_another_size_is_told_the_actual_size`.
- `an_app_without_an_event_channel_drops_pending_events`: merged into `pending_events_reach_the_event_channel_in_order_or_are_dropped_without_one`.
src/gamepad.rs
- `bumpers_and_triggers_follow_gilrs_numbering`, `every_engine_button_maps_to_gilrs_and_back`, `every_engine_axis_maps_to_gilrs_and_back`, `gilrs_inputs_the_engine_does_not_name_are_dropped`: merged into `every_engine_input_maps_to_gilrs_and_back_and_the_rest_is_dropped` (the right pair is pinned by the `RightTrigger2` row of the change table and the round trip over `GamepadButton::ALL`).
- `a_button_change_becomes_a_button_event_with_its_value`, `an_axis_change_keeps_its_value_and_sign`, `presses_releases_and_unnamed_inputs_translate_to_nothing`: rows of `only_a_named_button_or_axis_change_becomes_an_engine_event_with_its_value`.
Kept: `the_close_button_exits_unless_intercepted`, `the_left_button_is_the_mouse_left_or_the_pen_tip`, handler `dropping_the_handler_drops_its_app`.
BREAKS: key text filtering only '\r' (key table); a touch press as the left button (translate table); decorations applied only once something was applied (settings table); axis values losing their sign (gamepad table).

NOT VERIFIED: nothing. None of these crates draws.

CONCERNS:
- **Private, non-maths functions kept under test** because each table is the only cover of the behaviour: winit `translate`/`key`/`while_waiting`/`settings_actions`/`size_correction`, window `arboard_read` and `text_read`, foundation `content_root_for_exe` and `directory_of`. The reflect macro's `reflect_root` tests went the other way (the workspace build compiles every path).
- **Unit tests that still read private state.** The asset_server tests take tasks out of `pending_tasks` and wait on `asset_load_event_receiver` to drive loads deterministically; no `#[cfg(test)]` item remains, but they are module-internal. `uuid_requests_share_pending_and_loaded_assets` still counts `pending_tasks`.
- **Messages pinned harder.** The reflect refusal table pins the exact text of every row; before, about a third of the rows only checked `is_err`. A wording change now edits one table row.
- **Lockfile commit placement.** The one-line Cargo.lock change for name's proptest landed in the scene commit (ea880ea) rather than the name commit.
- **Clippy after commit.** a35a8ab was committed before its clippy run finished; 48b156a fixes the one lint (`err().expect` to `expect_err`).
- **Header comments.** I edited the `//!` header of foundation's asset_registry.rs, because the save tests it described moved out. Other existing `//!` headers were left as they were (spawn_scene.rs, content_asset_format.rs, event_lifetime.rs), though they now describe fewer or merged tests.
- **Tuples left in place.** Some production APIs return tuples: `decay_spring_damper_exact`, `take_requests`, `decode_content_asset`, the `(u32, u32)` window sizes, and ECS query items. The tests use these as given. Proptest strategies use `prop_compose!` so the tests define no tuple types.
- **derived_data failure table.** The derive-error row has no inputs read to check, so it carries a placeholder `Err` with no names. It reads a little oddly.
