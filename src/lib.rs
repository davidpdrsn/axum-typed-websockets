//! [`axum::extract::ws`] with type safe messages.
//!
//! # Example
//!
//! ```rust
//! use axum::{
//!     Router,
//!     response::IntoResponse,
//!     routing::get,
//! };
//! use axum_typed_websockets::{Message, WebSocket, WebSocketUpgrade};
//! use serde::{Serialize, Deserialize};
//! use std::time::Instant;
//!
//! // Make a regular axum router
//! let app = Router::new().route("/ws", get(handler));
//!
//! # async {
//! // Run it!
//! axum::serve(
//!     tokio::net::TcpListener::bind("0.0.0.0:3000")
//!         .await
//!         .unwrap(),
//!     app.into_make_service()
//! )
//! .await
//! .unwrap();
//! # };
//!
//! async fn handler(
//!     // Upgrade the request to a WebSocket connection where the server sends
//!     // messages of type `ServerMsg` and the clients sends `ClientMsg`
//!     ws: WebSocketUpgrade<ServerMsg, ClientMsg>,
//! ) -> impl IntoResponse {
//!     ws.on_upgrade(ping_pong_socket)
//! }
//!
//! // Send a ping and measure how long time it takes to get a pong back
//! async fn ping_pong_socket(mut socket: WebSocket<ServerMsg, ClientMsg>) {
//!     let start = Instant::now();
//!     socket.send(Message::Item(ServerMsg::Ping)).await.ok();
//!
//!     if let Some(msg) = socket.recv().await {
//!         match msg {
//!             Ok(Message::Item(ClientMsg::Pong)) => {
//!                 println!("ping: {:?}", start.elapsed());
//!             },
//!             Ok(_) => {},
//!             Err(err) => {
//!                 eprintln!("got error: {}", err);
//!             },
//!         }
//!     }
//! }
//!
//! #[derive(Debug, Serialize)]
//! enum ServerMsg {
//!     Ping,
//! }
//!
//! #[derive(Debug, Deserialize)]
//! enum ClientMsg {
//!     Pong,
//! }
//! ```
//!
//! # Feature flags
//!
//! The following features are available:
//!
//! - `json`: Enables [`JsonCodec`] which encodes message as JSON using
//! `serde_json`. Enabled by default.

#![warn(
    clippy::all,
    clippy::dbg_macro,
    clippy::todo,
    clippy::empty_enum,
    clippy::enum_glob_use,
    clippy::mem_forget,
    clippy::unused_self,
    clippy::filter_map_next,
    clippy::needless_continue,
    clippy::needless_borrow,
    clippy::match_wildcard_for_single_variants,
    clippy::if_let_mutex,
    clippy::mismatched_target_os,
    clippy::await_holding_lock,
    clippy::match_on_vec_items,
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    clippy::lossy_float_literal,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::fn_params_excessive_bools,
    clippy::exit,
    clippy::inefficient_to_string,
    clippy::linkedlist,
    clippy::macro_use_imports,
    clippy::option_option,
    clippy::verbose_file_reads,
    clippy::unnested_or_patterns,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
    missing_debug_implementations,
    missing_docs
)]
#![deny(unreachable_pub, private_interfaces, private_bounds)]
#![allow(elided_lifetimes_in_paths, clippy::type_complexity)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(test, allow(clippy::float_cmp))]

use axum::{
    async_trait,
    extract::{ws, FromRequestParts},
    http::request::Parts,
    response::IntoResponse,
};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    error::Error as StdError,
    fmt,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

#[allow(unused_macros)]
macro_rules! with_and_without_json {
    (
        $(#[$m:meta])*
        pub struct $name:ident<S, R, C = JsonCodec> {
            $(
                $ident:ident : $ty:ty,
            )*
        }
    ) => {
        $(#[$m])*
        #[cfg(feature = "json")]
        pub struct $name<S, R, C = JsonCodec> {
            $(
                $ident : $ty,
            )*
        }

        $(#[$m])*
        #[cfg(not(feature = "json"))]
        pub struct $name<S, R, C> {
            $(
                $ident : $ty,
            )*
        }
    }
}

with_and_without_json! {
    /// A version of [`axum::extract::ws::WebSocketUpgrade`] with type safe
    /// messages.
    ///
    /// # Type parameters
    ///
    /// - `S` - The message sent from the server to the client.
    /// - `R` - The message sent from the client to the server.
    /// - `C` - The [`Codec`] used to encode and decode messages. Defaults to
    /// [`JsonCodec`] which serializes messages with `serde_json`.
    pub struct WebSocketUpgrade<S, R, C = JsonCodec> {
        upgrade: ws::WebSocketUpgrade,
        _marker: PhantomData<fn() -> (S, R, C)>,
    }
}

#[async_trait]
impl<S, R, C, B> FromRequestParts<B> for WebSocketUpgrade<S, R, C>
where
    B: Send + Sync,
{
    type Rejection = <ws::WebSocketUpgrade as FromRequestParts<B>>::Rejection;

    async fn from_request_parts(parts: &mut Parts, state: &B) -> Result<Self, Self::Rejection> {
        let upgrade = FromRequestParts::from_request_parts(parts, state).await?;
        Ok(Self {
            upgrade,
            _marker: PhantomData,
        })
    }
}

impl<S, R, C> WebSocketUpgrade<S, R, C> {
    /// Finalize upgrading the connection and call the provided callback with
    /// the stream.
    ///
    /// This is analagous to [`axum::extract::ws::WebSocketUpgrade::on_upgrade`].
    pub fn on_upgrade<F, Fut>(self, callback: F) -> impl IntoResponse
    where
        F: FnOnce(WebSocket<S, R, C>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
        S: Send,
        R: Send,
    {
        self.upgrade
            .on_upgrade(|socket| async move {
                let socket = WebSocket {
                    socket,
                    _marker: PhantomData,
                };
                callback(socket).await
            })
            .into_response()
    }

    /// Apply a transformation to the inner [`axum::extract::ws::WebSocketUpgrade`].
    ///
    /// This can be used to apply configuration.
    pub fn map<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ws::WebSocketUpgrade) -> ws::WebSocketUpgrade,
    {
        self.upgrade = f(self.upgrade);
        self
    }

    /// Get the inner axum [`axum::extract::ws::WebSocketUpgrade`].
    pub fn into_inner(self) -> ws::WebSocketUpgrade {
        self.upgrade
    }
}

impl<S, R, C> fmt::Debug for WebSocketUpgrade<S, R, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSocketUpgrade")
            .field("upgrade", &self.upgrade)
            .finish()
    }
}

with_and_without_json! {
    /// A version of [`axum::extract::ws::WebSocket`] with type safe
    /// messages.
    pub struct WebSocket<S, R, C = JsonCodec> {
        socket: ws::WebSocket,
        _marker: PhantomData<fn() -> (S, R, C)>,
    }
}

impl<S, R, C> WebSocket<S, R, C> {
    /// Receive another message.
    ///
    /// Returns `None` if the stream stream has closed.
    ///
    /// This is analagous to [`axum::extract::ws::WebSocket::recv`] but with a
    /// statically typed message.
    pub async fn recv(&mut self) -> Option<Result<Message<R>, Error<C::Error>>>
    where
        R: DeserializeOwned,
        C: Codec,
    {
        self.next().await
    }

    /// Send a message.
    ///
    /// This is analagous to [`axum::extract::ws::WebSocket::send`] but with a
    /// statically typed message.
    pub async fn send(&mut self, msg: Message<S>) -> Result<(), Error<C::Error>>
    where
        S: Serialize,
        C: Codec,
    {
        SinkExt::send(self, msg).await
    }

    /// Gracefully close this WebSocket.
    ///
    /// This is analagous to [`axum::extract::ws::WebSocket::close`].
    pub async fn close(self) -> Result<(), Error<C::Error>>
    where
        C: Codec,
    {
        self.socket.close().await.map_err(Error::Ws)
    }

    /// Get the inner axum [`axum::extract::ws::WebSocket`].
    pub fn into_inner(self) -> ws::WebSocket {
        self.socket
    }
}

impl<S, R, C> fmt::Debug for WebSocket<S, R, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSocket")
            .field("socket", &self.socket)
            .finish()
    }
}

impl<S, R, C> Stream for WebSocket<S, R, C>
where
    R: DeserializeOwned,
    C: Codec,
{
    type Item = Result<Message<R>, Error<C::Error>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let msg = futures_util::ready!(Pin::new(&mut self.socket)
            .poll_next(cx)
            .map_err(Error::Ws)?);

        if let Some(msg) = msg {
            let msg = match msg {
                ws::Message::Text(msg) => msg.into_bytes(),
                ws::Message::Binary(bytes) => bytes,
                ws::Message::Close(frame) => {
                    return Poll::Ready(Some(Ok(Message::Close(frame))));
                }
                ws::Message::Ping(buf) => {
                    return Poll::Ready(Some(Ok(Message::Ping(buf))));
                }
                ws::Message::Pong(buf) => {
                    return Poll::Ready(Some(Ok(Message::Pong(buf))));
                }
            };

            let msg = C::decode(msg).map(Message::Item).map_err(Error::Codec);
            Poll::Ready(Some(msg))
        } else {
            Poll::Ready(None)
        }
    }
}

impl<S, R, C> Sink<Message<S>> for WebSocket<S, R, C>
where
    S: Serialize,
    C: Codec,
{
    type Error = Error<C::Error>;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.socket).poll_ready(cx).map_err(Error::Ws)
    }

    fn start_send(mut self: Pin<&mut Self>, msg: Message<S>) -> Result<(), Self::Error> {
        let msg = match msg {
            Message::Item(buf) => ws::Message::Binary(C::encode(buf).map_err(Error::Codec)?),
            Message::Ping(buf) => ws::Message::Ping(buf),
            Message::Pong(buf) => ws::Message::Pong(buf),
            Message::Close(frame) => ws::Message::Close(frame),
        };

        Pin::new(&mut self.socket)
            .start_send(msg)
            .map_err(Error::Ws)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.socket).poll_flush(cx).map_err(Error::Ws)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.socket).poll_close(cx).map_err(Error::Ws)
    }
}

/// Trait for encoding and decoding WebSocket messages.
///
/// This allows you to customize how messages are encoded when sent over the
/// wire.
pub trait Codec {
    /// The errors that can happen when using this codec.
    type Error;

    /// Encode a message.
    fn encode<S>(msg: S) -> Result<Vec<u8>, Self::Error>
    where
        S: Serialize;

    /// Decode a message.
    fn decode<R>(buf: Vec<u8>) -> Result<R, Self::Error>
    where
        R: DeserializeOwned;
}

/// A [`Codec`] that serializes messages as JSON using `serde_json`.
#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
#[derive(Debug)]
#[non_exhaustive]
pub struct JsonCodec;

#[cfg(feature = "json")]
#[cfg_attr(docsrs, doc(cfg(feature = "json")))]
impl Codec for JsonCodec {
    type Error = serde_json::Error;

    fn encode<S>(msg: S) -> Result<Vec<u8>, Self::Error>
    where
        S: Serialize,
    {
        serde_json::to_vec(&msg)
    }

    fn decode<R>(buf: Vec<u8>) -> Result<R, Self::Error>
    where
        R: DeserializeOwned,
    {
        serde_json::from_slice(&buf)
    }
}

/// Errors that can happen when using this library.
#[derive(Debug)]
pub enum Error<E> {
    /// Something went wrong with the WebSocket.
    Ws(axum::Error),
    /// Something went wrong with the [`Codec`].
    Codec(E),
}

impl<E> fmt::Display for Error<E>
where
    E: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Ws(inner) => inner.fmt(f),
            Error::Codec(inner) => inner.fmt(f),
        }
    }
}

impl<E> StdError for Error<E>
where
    E: StdError + 'static,
{
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Ws(inner) => Some(inner),
            Error::Codec(inner) => Some(inner),
        }
    }
}

/// A WebSocket message contain a value of a known type.
#[derive(Debug, Eq, PartialEq, Clone)]
pub enum Message<T> {
    /// An item of type `T`.
    Item(T),
    /// A ping message with the specified payload
    ///
    /// The payload here must have a length less than 125 bytes
    Ping(Vec<u8>),
    /// A pong message with the specified payload
    ///
    /// The payload here must have a length less than 125 bytes
    Pong(Vec<u8>),
    /// A close message with the optional close frame.
    Close(Option<ws::CloseFrame<'static>>),
}
