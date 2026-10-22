use std::{sync::Arc, thread::JoinHandle};

use concerto_app::App;
use crossbeam_channel::Sender;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{McpCall, McpInbox, Reply, ToolError, ToolRegistry, reply::Answer};

struct Handler {
    tools: Arc<Vec<Tool>>,
    inbox: Sender<McpCall>,
}

fn stopped() -> ToolError {
    ToolError::new("internal", "The app stopped before answering.")
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("concerto", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.as_ref().clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if !self.tools.iter().any(|tool| tool.name == request.name) {
            return Err(ErrorData::invalid_params(
                format!("No tool named `{}`.", request.name),
                None,
            ));
        }
        let (reply, answer) = Reply::new();
        let call = McpCall {
            tool: request.name.to_string(),
            arguments: Value::Object(request.arguments.unwrap_or_default()),
            reply,
        };
        let answer = match self.inbox.send(call) {
            Ok(()) => answer.await.unwrap_or_else(|_| Answer::Error(stopped())),
            Err(_) => Answer::Error(stopped()),
        };
        Ok(match answer {
            Answer::Content(content) => CallToolResult::success(content),
            Answer::Error(error) => CallToolResult::error(vec![error.content()]),
        }
        .into())
    }
}

/// The protocol thread, which ends when the client disconnects.
pub struct ServerHandle {
    thread: JoinHandle<()>,
}

impl ServerHandle {
    /// Whether the client has disconnected and the thread has ended.
    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Waits for the client to disconnect.
    pub fn join(self) {
        let _ = self.thread.join();
    }
}

/// Serves MCP over `io` on a new thread, offering the tools `app` has registered so far.
pub fn spawn_server<IO>(app: &App, io: IO) -> ServerHandle
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let handler = Handler {
        tools: Arc::new(
            app.get_resource::<ToolRegistry>()
                .expect("spawn_server needs McpPlugin")
                .descriptors(),
        ),
        inbox: app
            .get_resource::<McpInbox>()
            .expect("spawn_server needs McpPlugin")
            .sender(),
    };
    let thread = std::thread::Builder::new()
        .name("mcp-server".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the MCP server's runtime starts");
            runtime.block_on(async move {
                match handler.serve(io).await {
                    Ok(service) => {
                        if let Err(error) = service.waiting().await {
                            log::error!("MCP server stopped: {error}");
                        }
                    }
                    Err(error) => log::error!("MCP client failed to initialise: {error}"),
                }
            });
        })
        .expect("the MCP server thread starts");
    ServerHandle { thread }
}
