use crate::{
    model::{Health, Metric},
    plugins::{Collector, Reading},
};
use async_trait::async_trait;
use std::{collections::BTreeMap, fs, time::Instant};

#[derive(Default)]
pub struct SystemCollector {
    previous_cpu: Option<(u64, u64)>,
    previous_network: Option<(Instant, u64, u64)>,
}

fn cpu_ticks(text: &str) -> Option<(u64, u64)> {
    let values: Vec<u64> = text
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    let idle = values[3] + values.get(4).copied().unwrap_or(0);
    Some((values.iter().sum(), idle))
}
fn memory(text: &str) -> Option<(u64, u64)> {
    let values: BTreeMap<_, _> = text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?.parse::<u64>().ok()?))
        })
        .collect();
    let total = *values.get("MemTotal:")?;
    let available = *values.get("MemAvailable:")?;
    Some((total, available))
}
fn network(text: &str) -> (u64, u64) {
    text.lines()
        .filter_map(|line| {
            let (interface, values) = line.split_once(':')?;
            if interface.trim() == "lo" {
                return None;
            }
            let values: Vec<_> = values.split_whitespace().collect();
            Some((
                values.first()?.parse::<u64>().ok()?,
                values.get(8)?.parse::<u64>().ok()?,
            ))
        })
        .fold((0, 0), |(rx, tx), (a, b)| {
            (rx.saturating_add(a), tx.saturating_add(b))
        })
}

#[async_trait]
impl Collector for SystemCollector {
    async fn collect(&mut self) -> Reading {
        let Ok(mem) = fs::read_to_string("/proc/meminfo") else {
            return Reading::failed("系统指标需要 Linux /proc");
        };
        let Some((total, available)) = memory(&mem) else {
            return Reading::failed("无法读取内存指标");
        };
        let mut metrics = vec![
            Metric::new(
                "内存占用",
                format!("{:.1}", (total.saturating_sub(available)) as f64 / 1024.0),
                "MiB",
            ),
            Metric::new("内存总量", format!("{:.0}", total as f64 / 1024.0), "MiB"),
        ];
        let mut partial = false;
        if let Some(current) = fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| cpu_ticks(&s))
        {
            let usage = self.previous_cpu.and_then(|previous| {
                let total = current.0.checked_sub(previous.0)?;
                let idle = current.1.checked_sub(previous.1)?;
                (total > 0).then(|| {
                    format!(
                        "{:.1}",
                        100.0 * total.saturating_sub(idle) as f64 / total as f64
                    )
                })
            });
            let unit = if usage.is_some() { "%" } else { "" };
            metrics.push(Metric::new(
                "CPU",
                usage.unwrap_or_else(|| "采样中".into()),
                unit,
            ));
            self.previous_cpu = Some(current);
        } else {
            partial = true;
        }
        if let Ok(load) = fs::read_to_string("/proc/loadavg") {
            metrics.push(Metric::new(
                "1 分钟负载",
                load.split_whitespace().next().unwrap_or("—"),
                "",
            ));
        }
        if let Some(seconds) = fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        {
            let minutes = seconds as u64 / 60;
            metrics.push(Metric::new(
                "运行时间",
                format!(
                    "{}d {}h {}m",
                    minutes / 1440,
                    minutes / 60 % 24,
                    minutes % 60
                ),
                "",
            ));
        }
        if let Ok(text) = fs::read_to_string("/proc/net/dev") {
            let (rx, tx) = network(&text);
            let now = Instant::now();
            if let Some((previous, old_rx, old_tx)) = self.previous_network {
                let seconds = now.duration_since(previous).as_secs_f64().max(0.001);
                metrics.push(Metric::new(
                    "接口合计接收",
                    format!("{:.1}", rx.saturating_sub(old_rx) as f64 / seconds / 1024.0),
                    "KiB/s",
                ));
                metrics.push(Metric::new(
                    "接口合计发送",
                    format!("{:.1}", tx.saturating_sub(old_tx) as f64 / seconds / 1024.0),
                    "KiB/s",
                ));
            }
            self.previous_network = Some((now, rx, tx));
        }
        // Optional sensor: absence is normal on VMs and many boards.
        if let Some(temp) = fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
            .ok()
            .and_then(|s| s.trim().parse::<f64>().ok())
        {
            metrics.push(Metric::new(
                "温度 zone0",
                format!("{:.1}", temp / 1000.0),
                "°C",
            ));
        }
        Reading {
            health: if partial {
                Health::Degraded
            } else {
                Health::Healthy
            },
            summary: if partial {
                "部分系统指标不可用".into()
            } else {
                "系统指标采集正常 · 接口合计可能包含桥接重复计数".into()
            },
            metrics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_parsing_handles_guest_cpu_and_available_memory() {
        // Guest time is already included in user/nice and must not be counted twice.
        assert_eq!(
            cpu_ticks("cpu 10 20 30 40 5 6 7 8 100 200"),
            Some((126, 45))
        );
        assert_eq!(
            memory("MemTotal: 1000 kB\nMemFree: 100 kB\nMemAvailable: 400 kB"),
            Some((1000, 400))
        );
        assert_eq!(
            network("lo: 999 0 0 0 0 0 0 0 999\neth0: 100 0 0 0 0 0 0 0 200"),
            (100, 200)
        );
    }
}
