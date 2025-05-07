use async_trait::async_trait;
use bytes::Bytes;
use log::{debug, error, info};
use pingora::http::ResponseHeader;
use pingora::prelude::*;
use pingora::proxy::ProxyHttp;
use pingora::proxy::http_proxy_service;
use serde_json::{Result as SerdeResult, Value, from_slice, to_vec};
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;

// api.cloudflare.com/client/v4
// thewisenerd.io/cfp/client/v4

const PROXY_HOST: &str = "api.cloudflare.com";

pub struct ProxyServer(Arc<LoadBalancer<RoundRobin>>);
pub struct RequestContext {
    buffer: Vec<u8>,
    deprecations: Vec<Deprecation>,
}

#[derive(Debug)]
pub struct Deprecation20241130 {
    zone_id: String,
}

#[derive(Debug)]
enum Deprecation {
    D2024_11_30(Deprecation20241130),
}

fn path_validate(path: &str) -> Option<Deprecation> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 6 {
        return None;
    }
    if !(parts[0] == "" && parts[1] == "client" && parts[2] == "v4" && parts[3] == "zones") {
        return None;
    }
    if !(parts[5] == "dns_records") {
        return None;
    }
    Some(Deprecation::D2024_11_30(Deprecation20241130 {
        zone_id: parts[4].to_string(),
    }))
}

fn mutate2024_11_30(content: &[u8], zone_id: &str) -> SerdeResult<Vec<u8>> {
    info!("patching dns_records for zone_id {}", zone_id);

    let mut response: Value = from_slice(content)?;

    if let Some(result) = response.get_mut("result").and_then(|v| v.as_array_mut()) {
        for record in result {
            record["zone_id"] = Value::String(zone_id.to_string());
        }
    }

    to_vec(&response)
}

// TODO: handle SerdeResult vs Result
fn mutate(content: &[u8], deprecations: &[Deprecation]) -> SerdeResult<Vec<u8>> {
    let mut mutated = content.to_vec();
    for deprecation in deprecations {
        match deprecation {
            Deprecation::D2024_11_30(d) => {
                mutated = mutate2024_11_30(&mutated, &d.zone_id)?;
            }
        }
    }
    Ok(mutated)
}

#[async_trait]
impl ProxyHttp for ProxyServer {
    type CTX = RequestContext;

    fn new_ctx(&self) -> Self::CTX {
        RequestContext {
            buffer: Vec::new(),
            deprecations: Vec::new(),
        }
    }

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

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool>
    where
        Self::CTX: Send + Sync,
    {
        let path_and_query = session.req_header().uri.path_and_query().unwrap();
        let path = path_and_query.path();
        debug!("path: {}", path);

        if let Some(deprecation) = path_validate(path) {
            debug!("deprecation: {:?}", deprecation);
            ctx.deprecations.push(deprecation);
        }

        Ok(false)
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

    async fn response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()>
    where
        Self::CTX: Send + Sync,
    {
        upstream_response.insert_header(
            "x-server-source",
            "https://github.com/thewisenerd/cloudflare_deprecation_reverse_proxy",
        )?;

        if ctx.deprecations.is_empty() {
            return Ok(());
        }

        // Remove content-length because the size of the new body is unknown
        upstream_response.remove_header("Content-Length");
        upstream_response.insert_header("Transfer-Encoding", "Chunked")?;
        Ok(())
    }

    fn response_body_filter(
        &self,
        _session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> Result<Option<Duration>>
    where
        Self::CTX: Send + Sync,
    {
        if ctx.deprecations.is_empty() {
            return Ok(None);
        }

        debug!("response_body_filter+");

        if let Some(b) = body {
            ctx.buffer.extend(&b[..]);
            b.clear();
        }
        if end_of_stream {
            let mutated = mutate(&ctx.buffer, &ctx.deprecations).unwrap_or_else(|e| {
                error!("mutate error: {}", e);
                ctx.buffer.clone()
            });
            *body = Some(Bytes::from(mutated));
            ctx.buffer.clear();
        }
        Ok(None)
    }
}

// RUST_LOG=cloudflare_deprecation_reverse_proxy=debug,info
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
