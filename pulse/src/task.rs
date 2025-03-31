use async_trait::async_trait;

#[async_trait]
pub trait Runnable {
    async fn start(&self);
    fn name(&self) -> &str;
}
