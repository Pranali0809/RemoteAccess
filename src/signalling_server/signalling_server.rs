use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use futures::{stream::SplitSink, SinkExt};
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};
use futures::StreamExt;
use std::env;
use std::net::SocketAddr;
use log::{info, error};
use serde::Deserialize;
use serde_json;
use uuid::Uuid;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, MutexGuard};

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Messages {
    CreateSession,
    SessionID(String),
    JoinSession,
    SendSDP,
    SendICE,
    Disconnect,
}

enum Destination {
    SourcePeer,
    OtherPeer(String),
}

#[derive(Debug)]
struct Session {
    host: String,
    participant: Option<String>,
}

struct User {
    address: SocketAddr,
}

type Sessions = Arc<Mutex<HashMap<String, Session>>>;
type Users = Arc<Mutex<HashMap<String, User>>>;

async fn add_user_if_not_exists(users: Users, peer_addr: SocketAddr) -> Option<String> {
    let mut users_guard = users.lock().await;
    for (key, user) in users_guard.iter() {
        if user.address == peer_addr {
            info!("User already exists: {}", key);
            return Some(key.clone());
        }
    }
    let user_id = Uuid::new_v4().to_string();
    users_guard.insert(user_id.clone(), User { address: peer_addr });
    info!("New user added: {}", user_id);
    Some(user_id)
}

async fn create_session(activeSessions: Sessions, users: Users,user_id: String) -> String {
    let session_id = Uuid::new_v4().to_string();

    let session = Session {
        host: user_id,
        participant: None,
    };

    let mut sessions_guard = activeSessions.lock().await;
    sessions_guard.insert(session_id.clone(), session);

    info!("New session ID: {}", session_id);
    session_id
}

// async fn send_sessionid(
//     sender: Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>,
//     session_id: String,
// ) {
//     let mut sender_guard = sender.lock().await;
//     if let Err(e) = sender_guard.send(Message::Text(session_id)).await {
//         error!("Failed to send session ID: {}", e);
//     }
// }

async fn handle_message(
    sender: Arc<Mutex<SplitSink<WebSocketStream<TcpStream>, Message>>>,
    message: Messages,
    sessions: Sessions,
    users: Users,
    user_id: String,

) {
    let (message_to_send,destination)=match message {
        Messages::CreateSession => {
            let session_id = create_session(sessions.clone(), users, user_id).await;
            info!("Session created");
            (session_id,Destination::SourcePeer)
        }
        // Messages::JoinSession => {
        //     info!("Join Session");
        // }
        // Messages::SendSDP => {
        //     info!("Send SDP");
        // }
        // Messages::SendICE => {
        //     info!("Send ICE");
        // }
        // Messages::Disconnect => {
        //     info!("Disconnect Session");
        // }
        _ => {
            info!("Invalid message");
            return;
        }
    };

    // match destination {
    //     Destination::SourcePeer => {
            
    //                 let mut sender_guard = sender.lock().await;
    //                 if let Err(e) = sender_guard.send(message_to_send).await {
    //                     error!("Failed to send session ID: {}", e);
    //                 }
            
    //     }
    //     Destination::OtherPeer(peer_id) => {
    //     }
    // }
}

async fn handle_connection(stream: TcpStream, sessions: Sessions, users: Users) {
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => addr,
        Err(e) => {
            error!("Failed to get peer address: {}", e);
            return;
        }
    };

    let user_id = add_user_if_not_exists(users.clone(), peer_addr)
        .await
        .unwrap();
    let ws_stream = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("Error during the websocket handshake: {}", e);
            return;
        }
    };
    //bounded or unbounded needs to be studied
    let (sender, mut receiver) = ws_stream.split();
    let sender = Arc::new(Mutex::new(sender));

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                info!("Received message: {}", text);

                match serde_json::from_str::<Messages>(&text) {
                    Ok(parsed_message) => {
                        handle_message(sender.clone(), parsed_message, sessions.clone(), users.clone(),user_id.clone()).await;
                    }
                    Err(e) => {
                        error!(
                            "Failed to parse WebSocket message: {}. Message: {}",
                            e, text
                        );
                    }
                }
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket connection closed");
                break;
            }
            Ok(_) => {
                error!("Unsupported message type received");
            }
            Err(e) => {
                error!("Error receiving message: {}", e);
                break;
            }
        }
    }
}

pub async fn main() {
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let users: Users = Arc::new(Mutex::new(HashMap::new()));
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let addr: SocketAddr = addr.parse().expect("Invalid address");
    info!("Server is running on {}", addr);

    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_connection(stream, sessions.clone(), users.clone()));
    }
}

