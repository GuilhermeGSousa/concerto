//! Queries over every component on an entity that implements a given trait.
//!
//! Mark a trait with [`#[queryable]`](crate::queryable), register each component that
//! implements it, then query `All<&dyn Trait>`:
//!
//! ```
//! use concerto_ecs::{All, Component, Query, World, queryable};
//!
//! #[queryable]
//! trait Describe {
//!     fn describe(&self) -> String;
//! }
//!
//! #[derive(Component)]
//! struct Door;
//!
//! impl Describe for Door {
//!     fn describe(&self) -> String {
//!         "a door".into()
//!     }
//! }
//!
//! let mut world = World::new();
//! world.register_component_as::<dyn Describe, Door>();
//! world.spawn(Door);
//!
//! let mut query = world.query::<All<&dyn Describe>, ()>();
//! for describables in query.iter(&mut world) {
//!     for describable in describables {
//!         assert_eq!(describable.describe(), "a door");
//!     }
//! }
//! ```
//!
//! # Registration is per world
//!
//! Like components and scene types, implementors are registered on a [`World`], and a trait
//! query only visits implementors registered on the world it queries.

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    marker::PhantomData,
};

use crate::{
    World,
    archetype::Archetype,
    component::{Component, ComponentId},
    entity::Entity,
    query::{QueryData, ReadOnlyQueryData, world_query::WorldQuery},
    system::{access::SystemAccess, meta::SystemMetadata},
    table::TableRowIndex,
    world::UnsafeWorldCell,
};

/// A trait object type that can be queried with [`All`].
///
/// Implemented for `dyn Trait + 'a` by [`#[queryable]`](crate::queryable); there is no need
/// to implement it by hand. Every object lifetime is covered so that the elided
/// `&dyn Trait` in a system's signature (which means `&'x (dyn Trait + 'x)`) is accepted,
/// the same way `&T` queries are implemented for every lifetime.
///
/// # Safety
///
/// `Static` must be `Self` with a `'static` object lifetime, and `from_static` must return
/// its argument unchanged: trait queries dereference the pointer it returns.
pub unsafe trait QueryableTrait {
    /// `Self` with a `'static` object lifetime: the type implementors are registered under.
    type Static: ?Sized + 'static;

    /// Shortens the object lifetime of a pointer produced by a registered cast.
    fn from_static(ptr: *const Self::Static) -> *const Self;
}

/// Casts a component `C` to the trait object `Self`.
///
/// `#[queryable]` implements this for `dyn Trait` and every component that implements
/// `Trait`; [`World::register_component_as`] uses it to record `C` as an implementor.
pub trait ImplementedBy<C: Component>: QueryableTrait {
    fn cast(component: &C) -> &Self::Static;
}

/// One registered implementor of `Dyn`: which column to look in and how to view its values.
pub(crate) struct TraitImpl<Dyn: ?Sized> {
    component_id: ComponentId,
    cast: unsafe fn(*const u8) -> *const Dyn,
}

// Derived impls would require `Dyn: Clone`, which a trait object never is.
impl<Dyn: ?Sized> Clone for TraitImpl<Dyn> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Dyn: ?Sized> Copy for TraitImpl<Dyn> {}

impl<Dyn: ?Sized> TraitImpl<Dyn> {
    fn of<Trait, C>() -> Self
    where
        Trait: ImplementedBy<C, Static = Dyn> + ?Sized,
        C: Component,
    {
        Self {
            component_id: ComponentId::of::<C>(),
            cast: cast_ptr::<Trait, C>,
        }
    }
}

/// # Safety
///
/// `ptr` must point to a live, initialized `C`.
unsafe fn cast_ptr<Trait, C>(ptr: *const u8) -> *const Trait::Static
where
    Trait: ImplementedBy<C> + ?Sized,
    C: Component,
{
    Trait::cast(unsafe { &*ptr.cast::<C>() })
}

/// The implementors of every queryable trait registered on one [`World`].
#[derive(Default)]
pub(crate) struct TraitRegistry {
    /// Keyed by the trait object's `TypeId`; each value is a `Vec<TraitImpl<Dyn>>` that
    /// only ever grows, so a query state can remember a prefix of it by length.
    implementors: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    /// Bumped by every registration, so query state can skip the lookup when nothing changed.
    generation: usize,
}

impl TraitRegistry {
    /// Records `C` as an implementor of `Trait`. Registering the same pair again does nothing.
    pub(crate) fn register<Trait, C>(&mut self)
    where
        Trait: ImplementedBy<C> + ?Sized,
        C: Component,
    {
        let implementors = self
            .implementors
            .entry(TypeId::of::<Trait::Static>())
            .or_insert_with(|| Box::new(Vec::<TraitImpl<Trait::Static>>::new()))
            .downcast_mut::<Vec<TraitImpl<Trait::Static>>>()
            .expect("trait registry entry holds the implementors of another trait");

        if implementors
            .iter()
            .any(|existing| existing.component_id == ComponentId::of::<C>())
        {
            return;
        }

        implementors.push(TraitImpl::of::<Trait, C>());
        self.generation += 1;
    }

    fn implementors<Dyn: ?Sized + 'static>(&self) -> &[TraitImpl<Dyn>] {
        self.implementors
            .get(&TypeId::of::<Dyn>())
            .map(|implementors| {
                implementors
                    .downcast_ref::<Vec<TraitImpl<Dyn>>>()
                    .expect("trait registry entry holds the implementors of another trait")
                    .as_slice()
            })
            .unwrap_or(&[])
    }
}

/// Query state for a trait query: the implementors of `Dyn` known when it was last refreshed.
///
/// Implementors are only ever appended, so the state keeps their component ids (to match
/// archetypes, which happens without a world) and fetches visit the same-length prefix of
/// the world's list.
pub struct TraitQueryState<Dyn: ?Sized + 'static> {
    component_ids: Vec<ComponentId>,
    generation: usize,
    _marker: PhantomData<fn() -> *const Dyn>,
}

impl<Dyn: ?Sized + 'static> TraitQueryState<Dyn> {
    fn new(world: &World) -> Self {
        let registry = world.trait_registry();
        Self {
            component_ids: registry
                .implementors::<Dyn>()
                .iter()
                .map(|implementor| implementor.component_id)
                .collect(),
            generation: registry.generation,
            _marker: PhantomData,
        }
    }

    fn refresh(&mut self, world: &World) -> bool {
        if world.trait_registry().generation == self.generation {
            return false;
        }

        let refreshed = Self::new(world);
        let changed = refreshed.component_ids != self.component_ids;
        *self = refreshed;
        changed
    }

    fn matches(&self, archetype: &Archetype) -> bool {
        self.component_ids
            .iter()
            .any(|component_id| archetype.contains(*component_id))
    }
}

/// Fetches every component on an entity that implements a trait.
///
/// `All<&dyn Trait>` yields a [`TraitIter`] over the entity's implementors of `Trait`, in
/// no particular order. An entity matches if it has at least one.
///
/// The trait must be marked [`#[queryable]`](crate::queryable) and each implementor
/// registered with [`World::register_component_as`]; unregistered implementors are not
/// visited.
///
/// # Scheduling
///
/// Which components a trait query touches is only known at runtime, so a system holding
/// one is scheduled as if it read every component: it runs alongside systems that only
/// read, and is ordered against any system that writes a component.
pub struct All<Q> {
    _marker: PhantomData<Q>,
}

impl<Dyn: QueryableTrait + ?Sized> WorldQuery for All<&Dyn> {
    type State = TraitQueryState<Dyn::Static>;

    fn init_state(world: &mut World) -> Self::State {
        TraitQueryState::new(world)
    }

    fn matches(state: &Self::State, archetype: &Archetype) -> bool {
        state.matches(archetype)
    }

    fn refresh_state(state: &mut Self::State, world: &World) -> bool {
        state.refresh(world)
    }
}

impl<Dyn: QueryableTrait + ?Sized> QueryData for All<&Dyn> {
    type Item<'w> = TraitIter<'w, Dyn>;

    fn component_ids() -> Vec<ComponentId> {
        vec![]
    }

    fn fetch<'w>(
        state: &Self::State,
        world: UnsafeWorldCell<'w>,
        entity: Entity,
    ) -> Option<Self::Item<'w>> {
        let world = world.world();
        let location = world.entity_store().find_location(entity)?;
        let archetype = world.archetypes().get(location.archetype_index as usize)?;
        // The state was refreshed against this world when the query was created, and
        // implementors are only appended, so this prefix is exactly what it matched.
        let implementors =
            &world.trait_registry().implementors::<Dyn::Static>()[..state.component_ids.len()];
        let iter = TraitIter {
            archetype,
            row: location.row,
            impls: implementors.iter(),
            _marker: PhantomData,
        };

        // `get_entity` does not consult archetype matching, so an entity with no
        // implementor has to be turned away here.
        iter.clone().next().is_some().then_some(iter)
    }

    fn fill_access(_meta: &mut SystemMetadata, access: &mut SystemAccess) {
        access.read_all_components();
    }
}

impl<Dyn: QueryableTrait + ?Sized> ReadOnlyQueryData for All<&Dyn> {}

/// The components on one entity that implement `Dyn`, yielded by an [`All`] query.
pub struct TraitIter<'w, Dyn: QueryableTrait + ?Sized> {
    archetype: &'w Archetype,
    row: TableRowIndex,
    impls: std::slice::Iter<'w, TraitImpl<Dyn::Static>>,
    _marker: PhantomData<fn() -> *const Dyn>,
}

impl<Dyn: QueryableTrait + ?Sized> Clone for TraitIter<'_, Dyn> {
    fn clone(&self) -> Self {
        Self {
            archetype: self.archetype,
            row: self.row,
            impls: self.impls.clone(),
            _marker: PhantomData,
        }
    }
}

impl<'w, Dyn: QueryableTrait + ?Sized + 'w> Iterator for TraitIter<'w, Dyn> {
    type Item = &'w Dyn;

    fn next(&mut self) -> Option<Self::Item> {
        for implementor in self.impls.by_ref() {
            if let Some(ptr) = self
                .archetype
                .get_component_ptr(implementor.component_id, self.row)
            {
                // SAFETY: `ptr` points at this entity's `component_id` value, and `cast`
                // was built for exactly that component type.
                return Some(unsafe { &*Dyn::from_static((implementor.cast)(ptr)) });
            }
        }
        None
    }
}
