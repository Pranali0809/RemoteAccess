use tokio::net::{TcpListener, TcpStream};
use futures::SinkExt;
use tokio_tungstenite::{accept_async, tungstenite::protocol::Message};
use futures::StreamExt;
use std::net::SocketAddr;
use log::{info, error};
use serde::Deserialize;
use serde_json;
use uuid::Uuid;
use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc::UnboundedSender};



#[derive(Deserialize)]
#[serde(tag = "type")]

enum Messages {
    CreateSession,
    JoinSession { session_id: String },
    SessionJoinSuccess,
    SessionJoinFailure,
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
type PeerMap = Arc<Mutex<HashMap<SocketAddr, UnboundedSender<Message>>>>;

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

async fn create_session(active_sessions: Sessions,user_id: String) -> String {
    let session_id = Uuid::new_v4().to_string();

    let session = Session {
        host: user_id,
        participant: None,
    };

    let mut sessions_guard = active_sessions.lock().await;
    sessions_guard.insert(session_id.clone(), session);

    info!("New session ID: {}", session_id);
    // info!("Session structure: {}", sessions_guard.get(&session_id).unwrap().host);
    session_id
}

async fn join_session(active_sessions: Sessions, session_id: String,user_id:String) -> Result<String, String> {
    let mut sessions_guard = active_sessions.lock().await;
    let possible_session = sessions_guard.get_mut(&session_id);    
        match possible_session {
            None => {
                // error!("Session not found: {}", session_id);
                Err(format!("Session not found: {}", session_id))
            }
            Some(session) => {
                match &session.participant { // An & is needed because it did not allow me to move value out of session.participant since session is a mutable reference
                    None => {
                        session.participant = Some(user_id);
                        info!("User {:?} joined session: {}", session.participant, session_id);
                        Ok(session.host.clone())
                    }
                    Some(existing_participant) => {
                        match existing_participant == &user_id { //& is needed because can't compare `&std::string::String` with `std::string::String`
                        
                            true => {
                                // error!("User is already in the session: {}", session_id);
                                Err(format!("User is already in the session: {}", session_id))
                            }
                            false => {
                                error!("Session is full: {}", session_id);
                            Err(format!("Session is full: {}", session_id))
                            }
                        }
                       
                    }
                    
                    
                }
        }
        
    } 
}

async fn handle_message(

    message: Messages,
    sessions: Sessions,
    users: Users,
    user_id: String,
    peers: PeerMap,

) {
    let (message_to_send,destination)=match message {
        Messages::CreateSession => {
            let session_id = create_session(sessions.clone(), user_id.clone()).await;
            info!("Session created");
            //Destination is source as after creation of session session id needs to be sent to source peer                                                                                                                                                             
            (session_id,Destination::SourcePeer)
        }
        Messages::JoinSession { session_id } => {
            let result = join_session(sessions.clone(), session_id.clone(), user_id.clone()).await;
            match result {
                Ok(host) => {
                    // info!("Session joined successfully. Host: {}", host);
                    (session_id.clone(), Destination::OtherPeer(host))
                }
                Err(e) => {
                    error!("Failed to join session: {}", e);
                    (String::from("failed"), Destination::SourcePeer)
                }
            }
        }
        
        
        // Messages::Disconnect => {
        //     info!("Disconnect Session");
        // }
        _ => {
            info!("Invalid message");
            return;
        }
    };

    match destination {
        Destination::SourcePeer => {
            
            let peers = peers.lock().await;
            let users_lock=users.lock().await;
            let peer_address = users_lock.get(&user_id).unwrap().address;
            let tx = match peers.get(&peer_address) {
                Some(x) => x,
                None => {
                    error!("Peer with peer address {} dropped", peer_address);
                    return;
                }
            };

            
            let send_res = tx.send(Message::Text(message_to_send));
            if send_res.is_err() {
                error!("{}", format!("Error Sending {:?}", send_res))
            }
             
        }
        Destination::OtherPeer(peer_id) => {
            let peers = peers.lock().await;
            let users_lock=users.lock().await;
            let peer_address = users_lock.get(&peer_id).unwrap().address;
            let tx = match peers.get(&peer_address) {
                Some(x) => x,
                None => {
                    error!("Peer with peer address {} dropped", peer_address);
                    return;
                }
            };

            let send_res = tx.send(Message::Text(message_to_send));
            if send_res.is_err() {
                error!("{}", format!("Error Sending {:?}", send_res))
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, sessions: Sessions, users: Users, peers: PeerMap) {
    
    let peer_addr = match stream.peer_addr() { //Get remote address of the peer
        Ok(addr) => addr, //If successfully retrieved, it returns the address
        Err(e) => {
            error!("Failed to get peer address: {}", e);
            return;
        }
    };

    let user_id = add_user_if_not_exists(users.clone(), peer_addr)
        .await
        .unwrap(); //The unwrap method extracts the value inside a Result or Option if it is valid (Ok or Some).If the value is an error (Err) or None, it will panic and terminate the program.
    
    let ws_stream = match accept_async(stream).await { //Upgrades the TcpStream into a WebSocketStream
        Ok(ws) => ws,
        Err(e) => {
            error!("Error during the websocket handshake: {}", e);
            return;
        }
    };
    
    let (sender, mut receiver) = ws_stream.split(); // Sender: Used to send messages to the client. Receiver: Used to receive messages from the client.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel(); 
    // Creates an unbounded multi-producer, single-consumer (MPSC) channel.
    // tx: The sending end of the channel.
    // rx: The receiving end of the channel.
    // The tx will be used to send messages into the channel, which the receiving end (rx) will consume.
    peers.lock().await.insert(peer_addr, tx); //Stores tx to send to this peer
    
    let sender = Arc::new(Mutex::new(sender)); //Arc<Mutex<>> to allow shared ownership across threads/tasks while maintaining thread safety.
    
    //To handle outgoing messages
    tokio::spawn(async move {
        let mut rx = rx; //is necessary somehow find out why

        //listens for msgs
        while let Some(msg) = rx.recv().await {
            let mut sender_guard = sender.lock().await;
            //sends the message over websocket connection
            if let Err(e) = sender_guard.send(msg).await {
                error!("Failed to forward message: {}", e);
                break;
            }
        }
    });

    //receives msgs from websocket stream
    while let Some(msg) = receiver.next().await {
        match msg {
            //Message here is type of msg used by websocket(inbuilt) do not confuse with our Messages enum
            Ok(Message::Text(text)) => {
                info!("Received message: {}", text);
                //If msg is text than parse to Messages enum
                match serde_json::from_str::<Messages>(&text) {
                    Ok(parsed_message) => {
                        handle_message( parsed_message, sessions.clone(), users.clone(),user_id.clone(),peers.clone()).await;
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
    peers.lock().await.remove(&peer_addr);

}

pub async fn main() {
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new())); //shared HashMap to store session data
    let users: Users = Arc::new(Mutex::new(HashMap::new())); //users HashMap to store user data
    let peers: PeerMap = Arc::new(Mutex::new(HashMap::new()));
    let addr: SocketAddr = "127.0.0.1:8080".to_string().parse().expect("Invalid address"); //If parsing fails, it panics with the message "Invalid address."
    info!("Server is running on {}", addr);

    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");

    while let Ok((stream, _)) = listener.accept().await { //accept incoming connections.Each successful connection returns a stream
        tokio::spawn(handle_connection(stream, sessions.clone(), users.clone(), peers.clone())); //spawns a new asynchronous task to handle the connection
    }
}

