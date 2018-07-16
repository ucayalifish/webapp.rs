//! Everything related to the websocket connection

use actix::prelude::*;
use actix_web::{
    ws::{Message, ProtocolError, WebsocketContext},
    Binary,
};
use backend::{
    database::executor::{CreateSession, DeleteSession, UpdateSession},
    server::State,
    token::Token,
};
use failure::Error;
use futures::Future;
use protocol::{Login, Request, Response, ResponseError, Session};
use serde_cbor::{from_slice, ser::to_vec_packed};

/// The actual websocket
pub struct WebSocket;

impl Actor for WebSocket {
    type Context = WebsocketContext<Self, State>;
}

/// Handler for `Message`
impl StreamHandler<Message, ProtocolError> for WebSocket {
    fn handle(&mut self, msg: Message, context: &mut Self::Context) {
        match msg {
            Message::Binary(bin) => if let Err(e) = self.handle_request(&bin, context) {
                warn!("Unable to send response: {}", e);
            },
            Message::Close(reason) => {
                info!("Closing websocket connection: {:?}", reason);
                context.stop();
            }
            e => warn!("Got invalid message: {:?}", e),
        }
    }
}

impl WebSocket {
    pub fn new() -> Self {
        Self {}
    }

    fn handle_request(&mut self, data: &Binary, context: &mut WebsocketContext<Self, State>) -> Result<(), Error> {
        // Try to read the message
        let request: Request = from_slice(data.as_ref())?;

        // Check the request type
        match request {
            Request::Login(login) => {
                // Check if its a credential or token login type
                match login {
                    Login::Credentials {
                        username: u,
                        password: p,
                    } => {
                        let response = Response::Login(self.handle_request_login_credentials(&u, &p, context));

                        // Send the response to the websocket
                        self.send(context, &response)?;
                        Ok(())
                    }
                    Login::Session(s) => {
                        let response = Response::Login(self.handle_request_login_token(&s, context));

                        // Send the response to the websocket
                        self.send(context, &response)?;
                        Ok(())
                    }
                }
            }
            Request::Logout(s) => {
                let response = Response::Logout(self.handle_request_logout(s, context));

                // Send the response to the websocket
                self.send(context, &response)?;
                Ok(())
            }
        }
    }

    /// Serialize the data and send it to the websocket
    fn send(&self, context: &mut WebsocketContext<Self, State>, response: &Response) -> Result<(), Error> {
        context.binary(to_vec_packed(&response)?);
        Ok(())
    }

    fn handle_request_login_credentials(
        &mut self,
        username: &str,
        password: &str,
        context: &mut WebsocketContext<Self, State>,
    ) -> Result<Session, ResponseError> {
        debug!("User {} is trying to login", username);

        // Error if username and password are invalid
        if username.is_empty() || password.is_empty() || username != password {
            debug!("Wrong username or password");
            return Err(ResponseError::WrongUsernamePassword);
        }

        // Create a new session
        let session = context
            .state()
            .database
            .send(CreateSession(Token::create(username)?))
            .wait()
            .map_err(|_| ResponseError::Database)??;

        // Return the session
        Ok(session)
    }

    fn handle_request_login_token(
        &mut self,
        session: &Session,
        context: &mut WebsocketContext<Self, State>,
    ) -> Result<Session, ResponseError> {
        debug!("Session token {} wants to be renewed", session.token);

        // Try to verify and create a new session
        let new_session = context
            .state()
            .database
            .send(UpdateSession {
                old_token: session.token.to_owned(),
                new_token: Token::verify(&session.token)?,
            })
            .wait()
            .map_err(|_| ResponseError::Database)??;

        // Return the new session
        Ok(new_session)
    }

    fn handle_request_logout(
        &mut self,
        session: Session,
        context: &mut WebsocketContext<Self, State>,
    ) -> Result<(), ResponseError> {
        // Remove the session from the internal storage
        context
            .state()
            .database
            .send(DeleteSession(session.token))
            .wait()
            .map_err(|_| ResponseError::Database)??;

        Ok(())
    }
}
