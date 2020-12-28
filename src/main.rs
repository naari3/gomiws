use std::{
    collections::HashMap,
    env,
    io::Error as IoError,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_channel::mpsc::{unbounded, UnboundedSender};
use futures_util::{
    future::{self, select, Either},
    pin_mut,
    stream::TryStreamExt,
    StreamExt,
};

use tokio::net::{TcpListener, TcpStream};
use tokio_compat_02::FutureExt;
use tungstenite::protocol::Message;

#[macro_use]
extern crate dotenv_codegen;

type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

async fn handle_connection(peer_map: PeerMap, raw_stream: TcpStream, addr: SocketAddr) {
    println!("Incoming TCP connection from: {}", addr);

    let ws_stream = tokio_tungstenite::accept_async(raw_stream)
        .compat()
        .await
        .expect("Error during the websocket handshake occurred");
    println!("WebSocket connection established: {}", addr);

    // Insert the write part of this peer to the peer map.
    let (tx, rx) = unbounded();
    peer_map.lock().unwrap().insert(addr, tx);

    let (outgoing, incoming) = ws_stream.split();

    let broadcast_incoming = incoming.try_for_each(|msg| {
        println!(
            "Received a message from {}: {}",
            addr,
            msg.to_text().unwrap()
        );

        future::ok(())
    });

    let receive_from_others = rx.map(Ok).forward(outgoing);

    pin_mut!(broadcast_incoming, receive_from_others);
    future::select(broadcast_incoming, receive_from_others).await;

    println!("{} disconnected", &addr);
    peer_map.lock().unwrap().remove(&addr);
}

async fn update_image(peer_map: PeerMap) {
    let mut interval = tokio::time::interval(Duration::from_millis(1000));

    let token = egg_mode::Token::Access {
        consumer: egg_mode::KeyPair::new(
            dotenv!("TWITTER_CONSUMER_KEY").trim(),
            dotenv!("TWITTER_CONSUMER_SECRET").trim(),
        ),
        access: egg_mode::KeyPair::new(
            dotenv!("TWITTER_ACCESS_KEY").trim(),
            dotenv!("TWITTER_ACCESS_SECRET").trim(),
        ),
    };

    let mut stream = egg_mode::stream::sample(&token);

    let mut msg_fut = stream.try_next();
    let mut tick_fut = interval.next();

    let url = Arc::new(Mutex::new(String::from("hello")));

    loop {
        match select(msg_fut, tick_fut).compat().await {
            Either::Left((tweet, tick_fut_continue)) => {
                if let Ok(tweet) = tweet {
                    if let Some(tweet) = tweet {
                        if let egg_mode::stream::StreamMessage::Tweet(tweet) = tweet {
                            if let Some(media) = tweet.extended_entities {
                                for info in media.media {
                                    match info.media_type {
                                        egg_mode::entities::MediaType::Photo => {
                                            let mut url = url.lock().unwrap();
                                            *url = info.media_url_https;
                                        }
                                        // egg_mode::entities::MediaType::Gif => {}
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                } else {
                    break;
                }
                msg_fut = stream.try_next();
                tick_fut = tick_fut_continue;
            }
            Either::Right((_, msg_fut_continue)) => {
                let peers = peer_map.lock().unwrap();

                // We want to broadcast the message to everyone except ourselves.
                let broadcast_recipients = peers.iter().map(|(_, ws_sink)| ws_sink);

                println!("Current url is: {}", url.lock().unwrap().to_string());
                for recp in broadcast_recipients {
                    recp.unbounded_send(Message::Text(url.lock().unwrap().to_string()))
                        .unwrap();
                }

                msg_fut = msg_fut_continue;
                tick_fut = interval.next();
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), IoError> {
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:9002".to_string());

    let state = PeerMap::new(Mutex::new(HashMap::new()));

    // // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&addr).compat().await;
    let listener = try_socket.expect("Failed to bind");
    println!("Listening on: {}", addr);

    tokio::spawn(update_image(state.clone()));

    // Let's spawn the handling of each connection in a separate task.
    while let Ok((stream, addr)) = listener.accept().compat().await {
        tokio::spawn(handle_connection(state.clone(), stream, addr));
    }

    Ok(())
}
