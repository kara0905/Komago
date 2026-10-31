use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("erreur de base de données : {0}")]
    Database(#[from] rusqlite::Error),
    #[error("erreur de fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("valeur de paramètre invalide : {0}")]
    InvalidSetting(#[from] serde_json::Error),
    #[error("La base (version {found}) est plus récente que cette version de Komago (version {supported})")]
    DatabaseTooNew { found: i64, supported: i64 },
    #[error("répertoire de l'application introuvable : {0}")]
    AppDirectory(String),
    #[error("{0}")]
    Archive(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Network(String),
    #[error("les connexions externes sont désactivées")]
    NetworkDisabled,
    #[error("analyse annulée")]
    Cancelled,
    #[error("élément introuvable : {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, Error>;

// Les commandes Tauri renvoient leurs erreurs au frontend sous forme de texte.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
