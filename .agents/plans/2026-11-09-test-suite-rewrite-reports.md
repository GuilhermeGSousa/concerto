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
