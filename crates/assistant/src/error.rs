//! Plain-language errors; the UI shows `message` and `hint` verbatim.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AssistantError {
    #[error("The assistant model is not installed yet.")]
    ModelNotInstalled,
    #[error("Download problem: {0}")]
    Download(String),
    #[error("The downloaded model file is corrupt (checksum mismatch): {0}")]
    Checksum(String),
    #[error("The model could not be loaded: {0}")]
    Load(String),
    #[error("The model could not generate an answer: {0}")]
    Generation(String),
    #[error("Document conversion problem: {0}")]
    Convert(String),
    #[error("The assistant is built without an inference backend.")]
    NoBackend,
    #[error("{0}")]
    Other(String),
}

impl AssistantError {
    /// A short, actionable hint for the UI.
    pub fn hint(&self) -> String {
        match self {
            AssistantError::ModelNotInstalled => "Open the Assistant panel and choose \"Download model\" (about 0.5 GB, one time, stored locally).".into(),
            AssistantError::Download(_) => "Check the internet connection and retry; the download resumes from scratch.".into(),
            AssistantError::Checksum(_) => "Delete the model folder and download again.".into(),
            AssistantError::Load(_) => "The file may be incomplete; download the model again.".into(),
            AssistantError::Generation(_) => "Try a shorter question.".into(),
            AssistantError::Convert(_) => "Supported inputs: .md, .txt, .yaml, .json, .docx, .pdf.".into(),
            AssistantError::NoBackend => "Rebuild VeriModel with the 'candle' feature enabled.".into(),
            AssistantError::Other(_) => String::new(),
        }
    }
}

pub type AssistantResult<T> = Result<T, AssistantError>;

impl From<std::io::Error> for AssistantError {
    fn from(e: std::io::Error) -> Self {
        AssistantError::Other(e.to_string())
    }
}
