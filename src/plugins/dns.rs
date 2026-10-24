use crate::{
    config::PluginConfig,
    model::{Health, Metric},
    plugins::{Collector, Reading},
};
use async_trait::async_trait;
use hickory_proto::{
    op::{Message, MessageType, Query, ResponseCode},
    rr::{Name, RData, RecordType},
};
use std::{net::SocketAddr, time::Instant};
use tokio::net::UdpSocket;

pub struct DnsCollector {
    server: SocketAddr,
    name: String,
}
impl DnsCollector {
    pub fn new(p: &PluginConfig) -> Self {
        Self {
            server: p.dns_server.unwrap(),
            name: p.dns_name.clone().unwrap(),
        }
    }
    async fn lookup(&self) -> Result<Vec<String>, &'static str> {
        let mut name = Name::from_ascii(&self.name).map_err(|_| "DNS 探测域名无效")?;
        name.set_fqdn(true);
        let query = Query::query(name.clone(), RecordType::A);
        let mut message = Message::new();
        message
            .set_id(rand::random())
            .set_recursion_desired(true)
            .add_query(query.clone());
        let bytes = message.to_vec().map_err(|_| "DNS 请求编码失败")?;
        let socket = UdpSocket::bind(if self.server.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        })
        .await
        .map_err(|_| "DNS 探测 socket 创建失败")?;
        socket
            .connect(self.server)
            .await
            .map_err(|_| "DNS 服务器不可达")?;
        socket.send(&bytes).await.map_err(|_| "DNS 请求发送失败")?;
        let mut buffer = [0u8; 4096];
        let length = socket
            .recv(&mut buffer)
            .await
            .map_err(|_| "DNS 响应接收失败")?;
        let response = Message::from_vec(&buffer[..length]).map_err(|_| "DNS 响应格式无效")?;
        if response.id() != message.id()
            || response.message_type() != MessageType::Response
            || response.op_code() != message.op_code()
            || response.queries() != [query]
        {
            return Err("DNS 响应与请求不匹配");
        }
        if response.truncated() {
            return Err("DNS UDP 响应截断；本版探测不回退 TCP");
        }
        if response.response_code() != ResponseCode::NoError {
            return Err("DNS 服务器返回解析错误");
        }
        let mut answer_name = name;
        for _ in 0..8 {
            let target = response.answers().iter().find_map(|record| {
                if record.name() != &answer_name {
                    return None;
                }
                match record.data() {
                    RData::CNAME(target) => Some(target.0.clone()),
                    _ => None,
                }
            });
            match target {
                Some(target) => answer_name = target,
                None => break,
            }
        }
        let ips: Vec<_> = response
            .answers()
            .iter()
            .filter(|record| record.name() == &answer_name)
            .filter_map(|record| match record.data() {
                RData::A(ip) => Some(ip.to_string()),
                _ => None,
            })
            .take(8)
            .collect();
        if ips.is_empty() {
            return Err("DNS 响应没有 A 记录");
        }
        Ok(ips)
    }
}
#[async_trait]
impl Collector for DnsCollector {
    async fn collect(&mut self) -> Reading {
        let started = Instant::now();
        match self.lookup().await {
            Ok(ips) => Reading {
                health: Health::Healthy,
                summary: "指定 DNS 服务器实际解析成功".into(),
                metrics: vec![
                    Metric::new("响应时间", started.elapsed().as_millis(), "ms"),
                    Metric::new("解析结果", ips.join(", "), ""),
                ],
            },
            Err(reason) => Reading::failed(reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::{Record, rdata::A};
    #[tokio::test]
    async fn queries_configured_server_and_rejects_mismatched_response() {
        for valid in [true, false] {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let address = socket.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let mut buf = [0; 1024];
                let (length, peer) = socket.recv_from(&mut buf).await.unwrap();
                let query = Message::from_vec(&buf[..length]).unwrap();
                let mut response = Message::new();
                response
                    .set_id(if valid {
                        query.id()
                    } else {
                        query.id().wrapping_add(1)
                    })
                    .set_message_type(MessageType::Response)
                    .add_query(query.queries()[0].clone())
                    .add_answer(Record::from_rdata(
                        Name::from_ascii("example.com").unwrap(),
                        60,
                        RData::A(A::new(192, 0, 2, 1)),
                    ));
                socket
                    .send_to(&response.to_vec().unwrap(), peer)
                    .await
                    .unwrap();
            });
            let collector = DnsCollector {
                server: address,
                name: "example.com".into(),
            };
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(1), collector.lookup())
                    .await
                    .unwrap()
                    .is_ok(),
                valid
            );
            server.await.unwrap();
        }
    }
}
