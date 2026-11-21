use concerto_color::Color;
use concerto_ecs::{
    IntoSystem, System, World,
    events::{event_channel::EventChannel, event_cursor::EventCursor},
};
use concerto_ui::grid::{
    GridStep, UIGrid, UIGridCell, UIGridMove, UIGridMoved, grid_step, move_grid, paint_grid_cells,
};
use concerto_ui::material::UIMaterial;

struct Move {
    from: usize,
    step: GridStep,
    to: usize,
}

#[test]
fn a_step_moves_one_cell_and_holds_at_edges_and_past_a_short_last_row() {
    let moves = [
        Move {
            from: 0,
            step: GridStep::Right,
            to: 1,
        },
        Move {
            from: 2,
            step: GridStep::Right,
            to: 2,
        },
        Move {
            from: 4,
            step: GridStep::Left,
            to: 3,
        },
        Move {
            from: 4,
            step: GridStep::Up,
            to: 1,
        },
        Move {
            from: 4,
            step: GridStep::Down,
            to: 7,
        },
        Move {
            from: 5,
            step: GridStep::Down,
            to: 5,
        },
    ];
    for check in moves {
        assert_eq!(
            grid_step(check.from, 3, 8, check.step),
            check.to,
            "{:?} from {} in 8 cells, 3 across",
            check.step,
            check.from
        );
    }
    assert_eq!(
        grid_step(2, 1, 4, GridStep::Right),
        2,
        "one column has no sideways"
    );
    assert_eq!(grid_step(2, 1, 4, GridStep::Down), 3);
    assert_eq!(
        grid_step(0, 3, 0, GridStep::Down),
        0,
        "an empty grid stays put"
    );
}

#[test]
fn a_move_signal_moves_the_cursor_reports_it_and_recolours_the_cells() {
    let plain = Color::srgba(0.1, 0.1, 0.1, 1.0);
    let chosen = Color::srgba(0.9, 0.6, 0.1, 1.0);
    let mut world = World::new();
    world.insert_resource(EventChannel::<UIGridMoved>::new());
    world.add_listener(move_grid);
    let grid = world.spawn(UIGrid::new(2, 4));
    let cells: Vec<_> = (0..4)
        .map(|index| {
            world.spawn((
                UIGridCell {
                    grid,
                    index,
                    fill: plain,
                    selected_fill: chosen,
                },
                UIMaterial::flat(plain),
            ))
        })
        .collect();
    let mut paint = paint_grid_cells.into_system();
    paint.initialize(&mut world);
    let colours = |world: &World| -> Vec<bool> {
        cells
            .iter()
            .map(|cell| {
                world
                    .get_component_for_entity::<UIMaterial>(*cell)
                    .unwrap()
                    .color
                    == chosen.to_linear()
            })
            .collect()
    };

    paint.run_and_apply((), &mut world);
    assert_eq!(colours(&world), [true, false, false, false]);

    world.trigger(UIGridMove {
        grid,
        step: GridStep::Down,
    });
    world.trigger(UIGridMove {
        grid,
        step: GridStep::Down,
    });
    paint.run_and_apply((), &mut world);

    assert_eq!(colours(&world), [false, false, true, false]);
    let moved: Vec<UIGridMoved> = world
        .get_resource::<EventChannel<UIGridMoved>>()
        .unwrap()
        .read_from(&mut EventCursor::default())
        .copied()
        .collect();
    assert_eq!(
        moved,
        [UIGridMoved { grid, selected: 2 }],
        "the second move was held at the bottom and reports nothing"
    );
}
