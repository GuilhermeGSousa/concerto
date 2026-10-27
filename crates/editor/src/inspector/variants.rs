use std::any::Any;

use concerto_ecs::{Entity, command::CommandQueue};
use concerto_reflect::{Reflect, ReflectRef};
use concerto_ui::theme::UITheme;

use super::{EditContext, EditError, ErasedEditor, PropertyRowValue};

/// Picks the variant of a reflected enum; its fields follow as rows of their own.
pub(crate) struct VariantPicker;

/// Turns a reflected `Option` on and off; the value's fields follow as rows of their own.
pub(crate) struct OptionToggle;

fn active_variant<E: 'static>(value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
    let type_id = (value as &dyn Any).type_id();
    let ReflectRef::Enum(value) = value.reflect_ref() else {
        return Err(EditError::TypeMismatch);
    };
    Ok(PropertyRowValue::erased::<E, &'static str>(
        type_id,
        value.variant_name(),
    ))
}

impl ErasedEditor for VariantPicker {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        active_variant::<Self>(value)
    }
    fn build(
        &self,
        _: &mut CommandQueue,
        _: Entity,
        _: &PropertyRowValue,
        _: &UITheme,
    ) -> Result<(), EditError> {
        Ok(())
    }
    fn apply(&self, _: &mut dyn Reflect, _: &dyn Any, _: &EditContext) -> Result<(), EditError> {
        Err(EditError::Rejected)
    }
    fn follows_snapshot(&self) -> bool {
        true
    }
}

impl ErasedEditor for OptionToggle {
    fn snapshot(&self, value: &dyn Reflect) -> Result<PropertyRowValue, EditError> {
        active_variant::<Self>(value)
    }
    fn build(
        &self,
        _: &mut CommandQueue,
        _: Entity,
        _: &PropertyRowValue,
        _: &UITheme,
    ) -> Result<(), EditError> {
        Ok(())
    }
    fn apply(&self, _: &mut dyn Reflect, _: &dyn Any, _: &EditContext) -> Result<(), EditError> {
        Err(EditError::Rejected)
    }
    fn follows_snapshot(&self) -> bool {
        true
    }
}
