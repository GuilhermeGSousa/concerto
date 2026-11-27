//! A grid of cells walked by a cursor that its owner moves, so each player can have their own.
use concerto_color::Color;
use concerto_ecs::{
    Component, Entity, Event, Query,
    events::event_writer::EventWriter,
    signal::{On, Signal},
};

use crate::material::UIMaterial;

/// One move of a grid's cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridStep {
    /// One row up.
    Up,
    /// One row down.
    Down,
    /// One column left.
    Left,
    /// One column right.
    Right,
}

/// The cell reached from `index` by `step` in a grid of `count` cells, `columns` across.
///
/// The cursor holds at an edge, and where the cell below does not exist. An
/// `index` past the last cell counts as the last cell, and an empty grid
/// answers 0.
pub fn grid_step(index: usize, columns: usize, count: usize, step: GridStep) -> usize {
    let columns = columns.max(1);
    if count == 0 {
        return 0;
    }
    let index = index.min(count - 1);
    let column = index % columns;
    let moved = match step {
        GridStep::Up => index.checked_sub(columns),
        GridStep::Down => Some(index + columns),
        GridStep::Left => (column > 0).then(|| index - 1),
        GridStep::Right => (column + 1 < columns).then_some(index + 1),
    };
    moved.filter(|cell| *cell < count).unwrap_or(index)
}

/// A grid's shape and its cursor.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UIGrid {
    /// How many cells span a row.
    pub columns: usize,
    /// How many cells there are, filling rows from the top left.
    pub count: usize,
    /// The index of the cell the cursor is on.
    pub selected: usize,
}

impl UIGrid {
    /// A grid of `count` cells, `columns` across, with the cursor on the first.
    pub fn new(columns: usize, count: usize) -> Self {
        Self {
            columns,
            count,
            selected: 0,
        }
    }

    /// Puts the cursor on `selected`, or on the last cell when it is past the end.
    pub fn with_selected(mut self, selected: usize) -> Self {
        self.selected = selected.min(self.count.saturating_sub(1));
        self
    }

    /// Moves the cursor by `step`, returning false when it was held where it is.
    pub fn step(&mut self, step: GridStep) -> bool {
        let next = grid_step(self.selected, self.columns, self.count, step);
        let moved = next != self.selected;
        self.selected = next;
        moved
    }
}

/// A cell of `grid`, coloured by whether the grid's cursor is on it.
///
/// Its `UIMaterial` takes `selected_fill` while the cursor is on `index`, and
/// `fill` otherwise.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct UIGridCell {
    /// The [`UIGrid`] entity.
    pub grid: Entity,
    /// The cell's index in the grid.
    pub index: usize,
    /// The colour while the cursor is elsewhere.
    pub fill: Color,
    /// The colour while the cursor is on the cell.
    pub selected_fill: Color,
}

/// Moves the cursor of `grid` by `step`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UIGridMove {
    /// The [`UIGrid`] entity.
    pub grid: Entity,
    /// The move.
    pub step: GridStep,
}
impl Signal for UIGridMove {}

/// The cursor of `grid` moved to `selected`.
#[derive(Event, Clone, Copy, Debug, PartialEq, Eq)]
pub struct UIGridMoved {
    /// The [`UIGrid`] entity.
    pub grid: Entity,
    /// The index of the cell the cursor is now on.
    pub selected: usize,
}

/// Applies a `UIGridMove` and reports the move.
pub fn move_grid(
    on: On<UIGridMove>,
    mut grids: Query<&mut UIGrid>,
    mut moved: EventWriter<UIGridMoved>,
) {
    let request = on.signal();
    if let Some(mut grid) = grids.get_entity_mut(request.grid)
        && grid.step(request.step)
    {
        moved.write(UIGridMoved {
            grid: request.grid,
            selected: grid.selected,
        });
    }
}

/// Colours each cell by whether its grid's cursor is on it.
pub fn paint_grid_cells(grids: Query<&UIGrid>, mut cells: Query<(&UIGridCell, &mut UIMaterial)>) {
    for (cell, mut material) in cells.iter_mut() {
        let chosen = grids
            .get_entity(cell.grid)
            .is_some_and(|grid| grid.selected == cell.index);
        let colour = if chosen {
            cell.selected_fill
        } else {
            cell.fill
        }
        .to_linear();
        if material.color != colour {
            material.color = colour;
        }
    }
}
