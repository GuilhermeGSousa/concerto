//! The protocol thread: rmcp on a single-threaded tokio runtime, forwarding
//! every tool call to the main thread and awaiting its answer.
use std::{sync::Arc, thread::JoinHandle};

use crossbeam_channel::Sender;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{McpInbox, McpRequest, McpTools, Message, ToolError};

/// How the server introduces itself to a client.
#[derive(Clone, Debug)]
pub struct ServerIdentity {
    pub name: String,
    pub version: String,
    /// Shown to the agent once, at connection: what the app is and how to start.
    pub instructions: String,
}

struct Handler {
    identity: ServerIdentity,
    tools: Arc<Vec<Tool>>,
    inbox: Sender<Message>,
}

impl ServerHandler for Handler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                self.identity.name.clone(),
                self.identity.version.clone(),
            ))
            .with_instructions(self.identity.instructions.clone())
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
        let (reply, answer) = tokio::sync::oneshot::channel();
        let arguments = request.arguments.map(Value::Object).unwrap_or_else(|| {
            // A call with no arguments means "all defaults".
            Value::Object(Default::default())
        });
        let sent = self.inbox.send(Message::Call(McpRequest {
            tool: request.name.to_string(),
            arguments,
            reply,
        }));
        let result = match sent {
            Ok(()) => answer.await.unwrap_or_else(|_| {
                Err(ToolError::new(
                    "shutting_down",
                    "The app stopped before answering.",
                ))
            }),
            Err(_) => Err(ToolError::new("shutting_down", "The app has stopped.")),
        };
        Ok(match result {
            Ok(output) => CallToolResult::success(vec![ContentBlock::text(output.text)]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        }
        .into())
    }
}

/// Serves MCP over `io` on a new thread. When the client disconnects the inbox
/// is told, so a runner waiting on it can exit.
pub fn spawn_server<IO>(
    identity: ServerIdentity,
    tools: &McpTools,
    inbox: &McpInbox,
    io: impl FnOnce() -> IO + Send + 'static,
) -> JoinHandle<()>
where
    IO: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let handler = Handler {
        identity,
        tools: Arc::new(tools.descriptors()),
        inbox: inbox.sender(),
    };
    let closed = inbox.sender();
    std::thread::Builder::new()
        .name("mcp-server".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("could not start the MCP server's runtime");
            runtime.block_on(async move {
                match handler.serve(io()).await {
                    Ok(service) => {
                        if let Err(error) = service.waiting().await {
                            log::error!("MCP server stopped: {error}");
                        }
                    }
                    Err(error) => log::error!("MCP client failed to initialise: {error}"),
                }
            });
            let _ = closed.send(Message::Closed);
        })
        .expect("could not spawn the MCP server thread")
}

/// Serves MCP over this process's stdin and stdout. Stdout then belongs to the
/// protocol: nothing else may print to it.
pub fn spawn_stdio_server(
    identity: ServerIdentity,
    tools: &McpTools,
    inbox: &McpInbox,
) -> JoinHandle<()> {
    spawn_server(identity, tools, inbox, || {
        tokio::io::join(tokio::io::stdin(), tokio::io::stdout())
    })
}
