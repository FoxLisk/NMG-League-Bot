
#[derive(Clone, Debug)]
/// Drop this struct to signal you are done shutting down
pub struct Shutdown {
    pub(crate) _handle: tokio::sync::mpsc::Sender<()>,
}