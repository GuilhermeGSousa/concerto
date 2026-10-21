use std::{any::TypeId, collections::HashMap};

use concerto_app::App;
use concerto_ecs::{
    IntoSystemConfig, Resource, System,
    system::{IntoSystem, access::SystemAccess, meta::SystemMetadata},
};
use rmcp::{
    handler::server::common::{schema_for_input, schema_for_type},
    model::{Tool, ToolAnnotations},
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{McpRead, McpServe, ToolError, answer_unanswered, receive_read_only_calls};

/// What a tool needs before it can answer; a call no system answered gets the error its need names.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Needs {
    None,
    Project,
    Scene,
    Playing,
}

/// How a tool is offered: its name, whether it only reads, and what it needs.
#[derive(Clone, Debug)]
pub struct McpTool {
    name: &'static str,
    read_only: bool,
    needs: Needs,
}

impl McpTool {
    /// A tool that may change something; its calls are answered in frames.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            read_only: false,
            needs: Needs::None,
        }
    }

    /// A tool that only reads; its calls are answered between frames too.
    pub fn read_only(name: &'static str) -> Self {
        Self {
            read_only: true,
            ..Self::new(name)
        }
    }

    /// Declares what the tool needs before it can answer.
    pub fn needs(mut self, needs: Needs) -> Self {
        self.needs = needs;
        self
    }
}

struct RegisteredTool {
    tool: McpTool,
    descriptor: Tool,
}

/// Every tool added with [`AppMcpExt::add_tool`], and the errors that unmet needs answer.
#[derive(Resource, Default)]
pub struct ToolRegistry {
    tools: Vec<RegisteredTool>,
    by_type: HashMap<TypeId, usize>,
    unmet: HashMap<Needs, ToolError>,
}

impl ToolRegistry {
    fn add<A: JsonSchema + 'static>(&mut self, tool: McpTool) {
        assert!(
            self.find(tool.name).is_none(),
            "MCP tool `{}` is added twice",
            tool.name
        );
        assert!(
            !self.by_type.contains_key(&TypeId::of::<A>()),
            "the arguments of MCP tool `{}` belong to another tool",
            tool.name
        );
        let description = schema_for_type::<A>()
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let schema = schema_for_input::<A>()
            .unwrap_or_else(|error| panic!("MCP tool `{}`: {error}", tool.name));
        let mut descriptor = Tool::new(tool.name, description, schema);
        descriptor.annotations = Some(ToolAnnotations::new().read_only(tool.read_only));
        self.by_type.insert(TypeId::of::<A>(), self.tools.len());
        self.tools.push(RegisteredTool { tool, descriptor });
    }

    fn find(&self, name: &str) -> Option<&McpTool> {
        self.tools
            .iter()
            .map(|registered| &registered.tool)
            .find(|tool| tool.name == name)
    }

    /// Whether the tool named `name` was added read-only.
    pub fn is_read_only(&self, name: &str) -> bool {
        self.find(name).is_some_and(|tool| tool.read_only)
    }

    pub(crate) fn name_of<A: 'static>(&self) -> Option<&'static str> {
        self.by_type
            .get(&TypeId::of::<A>())
            .map(|index| self.tools[*index].tool.name)
    }

    pub(crate) fn descriptors(&self) -> Vec<Tool> {
        self.tools
            .iter()
            .map(|registered| registered.descriptor.clone())
            .collect()
    }

    pub(crate) fn unanswered(&self, name: &str) -> ToolError {
        self.find(name)
            .and_then(|tool| self.unmet.get(&tool.needs))
            .cloned()
            .unwrap_or_else(|| ToolError::new("internal", format!("No system answered `{name}`.")))
    }
}

fn assert_reads_only<M>(name: &str, system: impl IntoSystem<(), M>) {
    let system = system.into_system();
    let mut access = SystemAccess::default();
    system.fill_access(&mut SystemMetadata::default(), &mut access);
    let mut everything = SystemAccess::default();
    everything.read_world();
    assert!(
        SystemAccess::are_disjoint(&access, &everything) && !access.needs_apply(),
        "read-only tool `{name}` has a system that writes: {}",
        system.name()
    );
}

/// Adds MCP tools to an `App` that has [`McpPlugin`](crate::McpPlugin).
pub trait AppMcpExt {
    /// Registers tool `A` and adds `system` to [`McpServe`], and also to [`McpRead`] for a read-only tool, whose system must only read.
    fn add_tool<A, M>(
        &mut self,
        tool: McpTool,
        system: impl IntoSystem<(), M> + Clone + 'static,
    ) -> &mut Self
    where
        A: DeserializeOwned + JsonSchema + 'static;

    /// Sets the error that a call answers when its tool needs `need` and no system answered it.
    fn describe_need(&mut self, need: Needs, error: ToolError) -> &mut Self;
}

impl AppMcpExt for App {
    fn add_tool<A, M>(
        &mut self,
        tool: McpTool,
        system: impl IntoSystem<(), M> + Clone + 'static,
    ) -> &mut Self
    where
        A: DeserializeOwned + JsonSchema + 'static,
    {
        let read_only = tool.read_only;
        if read_only {
            assert_reads_only(tool.name, system.clone());
        }
        self.get_resource_mut::<ToolRegistry>()
            .expect("register McpPlugin before adding tools")
            .add::<A>(tool);
        self.add_system(McpServe, system.clone().before(answer_unanswered));
        if read_only {
            self.add_system(
                McpRead,
                system
                    .after(receive_read_only_calls)
                    .before(answer_unanswered),
            );
        }
        self
    }

    fn describe_need(&mut self, need: Needs, error: ToolError) -> &mut Self {
        self.get_resource_mut::<ToolRegistry>()
            .expect("register McpPlugin before describing needs")
            .unmet
            .insert(need, error);
        self
    }
}
