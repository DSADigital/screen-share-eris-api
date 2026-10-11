//! Handlers HTTP/WebSocket — espelham `src/services/screen_share/services/*`
//! da Éris API. Diferenças em relação a produção estão comentadas em cada
//! arquivo com o marcador `[DEMO]`.

pub mod close_session;
pub mod create_session;
pub mod get_session;
pub mod identity;
pub mod join_session;
pub mod leave_session;
pub mod ws_signaling;

use actix_web::{
    HttpRequest,
    web::{ServiceConfig, scope},
};

pub fn routes(config: &mut ServiceConfig) {
    let scope = scope("/api/screen-share")
        .service(create_session::create_session)
        .service(get_session::get_session)
        .service(join_session::join_session)
        .service(leave_session::leave_session)
        .service(close_session::close_session)
        .service(ws_signaling::ws_signaling);

    config
        .service(identity::create_identity)
        .service(scope);
}

/// Monta a URL do WebSocket a partir do host da requisição (mesma lógica do original).
pub(crate) fn websocket_url(req: &HttpRequest) -> String {
    let conn_info = req.connection_info();
    let host = conn_info.host();
    let scheme = if conn_info.scheme() == "https" { "wss" } else { "ws" };
    format!("{}://{}/api/screen-share/ws", scheme, host)
}
