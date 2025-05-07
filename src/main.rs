use async_trait::async_trait;
use pingora::prelude::*;
use pingora::proxy::http_proxy_service;
use pingora::proxy::ProxyHttp;
use std::net::ToSocketAddrs;

const IP4_ME_IP: &str = "1.1.1.1";
const IP4_ME_PORT: u16 = 443;
const IP4_ME_HOST: &str = "one.one.one.one";

pub struct ProxyServer {
    addr: std::net::SocketAddr,
}

#[async_trait]
impl ProxyHttp for ProxyServer {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let peer = Box::new(HttpPeer::new(self.addr, true, IP4_ME_HOST.to_string()));
        println!("upstream_peer: {:?}", peer);
        Ok(peer)
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()>
    where
        Self::CTX: Send + Sync,
    {
        upstream_request.insert_header("Host", IP4_ME_HOST.to_string())?;
        Ok(())
    }

    async fn logging(
        &self,
        session: &mut Session,
        _e: Option<&pingora::Error>,
        ctx: &mut Self::CTX,
    ) {
        let response_code = session
            .response_written()
            .map_or(0, |resp| resp.status.as_u16());
        // access log
        println!(
            "{} response code: {response_code}",
            self.request_summary(session, ctx)
        );
    }
}

fn main() {
    let mut server = Server::new(Some(Opt::default())).unwrap();
    server.bootstrap();

    let mut http_proxy = http_proxy_service(
        &server.configuration,
        ProxyServer {
            addr: (IP4_ME_IP, IP4_ME_PORT)
                .to_socket_addrs()
                .unwrap()
                .next()
                .unwrap(),
        },
    );
    http_proxy.add_tcp("0.0.0.0:6191");

    server.add_service(http_proxy);
    println!("starting server!");
    server.run_forever();
}
