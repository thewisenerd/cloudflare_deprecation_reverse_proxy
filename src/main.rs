use async_trait::async_trait;
use bytes::Bytes;
use log::info;
use pingora::prelude::*;
use pingora::proxy::ProxyHttp;
use pingora::proxy::http_proxy_service;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;

const PROXY_HOST: &str = "api.cloudflare.com";

pub struct ProxyServer(Arc<LoadBalancer<RoundRobin>>);

#[async_trait]
impl ProxyHttp for ProxyServer {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let upstream = self.0.select(b"", 256).unwrap();
        info!("upstream: {:?}", upstream);
        let peer = Box::new(HttpPeer::new(upstream, true, PROXY_HOST.to_owned()));
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> Result<()>
    where
        Self::CTX: Send + Sync,
    {
        upstream_request.insert_header("Host", PROXY_HOST.to_owned())?;
        Ok(())
    }
}

fn main() {
    env_logger::init();

    let mut server = Server::new(Some(Opt::default())).unwrap();
    server.bootstrap();

    let addrs = (PROXY_HOST, 443).to_socket_addrs().unwrap();
    for addr in addrs.clone() {
        info!("upstream_addr: {:?}", addr);
    }

    let upstreams: LoadBalancer<RoundRobin> = LoadBalancer::try_from_iter(addrs).unwrap();

    let mut http_proxy =
        http_proxy_service(&server.configuration, ProxyServer(Arc::new(upstreams)));
    http_proxy.add_tcp("0.0.0.0:6191");

    server.add_service(http_proxy);
    server.run_forever()
}
