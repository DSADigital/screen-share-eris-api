//! Identidade efêmera da demo.
//!
//! Na Éris API, o extractor `AuthenticatedUser` valida um JWT e carrega o
//! usuário do PostgreSQL. Aqui, para que qualquer pessoa consiga rodar o
//! projeto sem banco nem conta, cada aba do navegador gera uma identidade
//! descartável: um `user_id` aleatório + um token opaco guardados SOMENTE em
//! memória. Reiniciar o servidor apaga tudo.
//!
//! O extractor mantém a mesma forma de uso do original (`user.0.id`,
//! `user.0.username`, `user.0.first_name`) para que os handlers fiquem
//! praticamente idênticos aos de produção.

use std::{
    collections::HashMap,
    future::{Ready, ready},
    sync::{Arc, RwLock},
};

use actix_web::{FromRequest, HttpRequest, dev::Payload, error::InternalError, web::Data};
use uuid::Uuid;

use crate::responses::unauthorized;

/// Limite de identidades simultâneas em memória (proteção simples contra abuso).
const MAX_IDENTITIES: usize = 10_000;

#[derive(Debug, Clone)]
pub struct DemoUser {
    pub id: Uuid,
    pub username: String,
    pub first_name: String,
}

#[derive(Clone, Default)]
pub struct IdentityStore {
    by_token: Arc<RwLock<HashMap<String, DemoUser>>>,
}

impl IdentityStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cria uma identidade descartável e devolve `(token, usuário)`.
    pub fn issue(&self, first_name: String) -> Option<(String, DemoUser)> {
        let mut map = self.by_token.write().ok()?;
        if map.len() >= MAX_IDENTITIES {
            return None;
        }

        let id = Uuid::new_v4();
        let username = format!("guest_{}", &id.simple().to_string()[..8]);
        let user = DemoUser {
            id,
            username,
            first_name,
        };
        let token = nanoid::nanoid!(40);
        map.insert(token.clone(), user.clone());
        Some((token, user))
    }

    pub fn resolve(&self, token: &str) -> Option<DemoUser> {
        self.by_token.read().ok()?.get(token).cloned()
    }
}

/// Equivalente demo de `middlewares::authenticated_user::AuthenticatedUser`.
pub struct AuthenticatedUser(pub DemoUser);

impl FromRequest for AuthenticatedUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let deny = || InternalError::from_response("unauthorized", unauthorized()).into();

        let Some(store) = req.app_data::<Data<IdentityStore>>() else {
            return ready(Err(deny()));
        };

        let token = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::trim);

        match token.and_then(|t| store.resolve(t)) {
            Some(user) => ready(Ok(AuthenticatedUser(user))),
            None => ready(Err(deny())),
        }
    }
}
