use concerto_animation::retarget::RetargetRig;
use concerto_ecs::{
    Added, Entity, Query, Res,
    command::CommandQueue,
    signal::{
        On,
        listener::{IntoListener, Listener},
    },
};
use concerto_editor::asset_editor::{EditorDocument, EditorHosts};
use concerto_foundation::assets::Asset;
use concerto_ui::{elements::prelude::*, interaction::UIClick, node::UINode, theme::UITheme};
use concerto_window::input::MouseButton;
use taffy::Position;

use crate::create::{ChainNames, CreateRagdoll};

fn create_for(document: Entity) -> Listener<UIClick> {
    (move |on: On<UIClick>, documents: Query<&EditorDocument>, mut cmd: CommandQueue| {
        if on.signal().button != MouseButton::Left {
            return;
        }
        let Some(rig) = documents
            .get_entity(document)
            .and_then(|shown| shown.current.as_ref())
            .map(|entry| entry.id)
        else {
            return;
        };
        cmd.trigger(CreateRagdoll {
            rig,
            address: None,
            total_mass: None,
            chains: ChainNames::default(),
        });
    })
    .into_listener()
}

pub(crate) fn add_create_buttons(
    documents: Query<(Entity, &EditorDocument, &EditorHosts), Added<EditorHosts>>,
    nodes: Query<&UINode>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (document, editor, hosts) in documents.iter() {
        let Some(inspector) = hosts
            .inspector
            .filter(|_| editor.asset_type == RetargetRig::name())
            .filter(|host| {
                nodes
                    .get_entity(*host)
                    .is_some_and(|node| node.position != Position::Absolute)
            })
        else {
            continue;
        };
        cmd.entity(inspector).spawn_child(
            theme
                .button("Create ragdoll")
                .on_click(create_for(document)),
        );
    }
}
