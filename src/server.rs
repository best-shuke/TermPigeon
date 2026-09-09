use std::{collections::HashSet, fs, process::Command};

pub fn server_status(has_session: bool) -> String {
    let hostname = fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "unknown".to_owned())
        .trim()
        .to_owned();
    let uptime = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok())
        .map(format_uptime)
        .unwrap_or_else(|| "unknown".to_owned());
    let load = fs::read_to_string("/proc/loadavg")
        .ok()
        .map(|value| {
            value
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_else(|| "unknown".to_owned());
    let memory = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|value| memory_summary(&value))
        .unwrap_or_else(|| "unknown".to_owned());
    let disk = command_output("df", &["-h", "--output=size,used,avail,pcent", "/"])
        .and_then(|value| value.lines().last().map(str::trim).map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    let service_count = command_output(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--state=running",
            "--no-legend",
            "--plain",
        ],
    )
    .map(|value| value.lines().filter(|line| !line.trim().is_empty()).count())
    .unwrap_or(0);
    let container_count = command_output("docker", &["ps", "-q"])
        .map(|value| value.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0);

    format!(
        "## 🖥️ 服务器状态\n\n- **主机**：`{hostname}`\n- **运行时间**：{uptime}\n- **负载（1/5/15 分钟）**：`{load}`\n- **内存**：{memory}\n- **根分区（总量/已用/可用/占用）**：`{disk}`\n- **运行中的 systemd 服务**：{service_count}\n- **运行中的容器**：{container_count}\n- **Codex 会话**：{}",
        if has_session {
            "已建立"
        } else {
            "尚未建立"
        }
    )
}

pub fn deployed_services() -> String {
    let mut lines = vec![
        "## 🧩 服务器服务".to_owned(),
        String::new(),
        "### 应用与网络服务".to_owned(),
    ];
    let running = command_output(
        "systemctl",
        &[
            "list-units",
            "--type=service",
            "--state=running",
            "--no-legend",
            "--plain",
        ],
    )
    .unwrap_or_default();
    let mut visible_count = 0;
    for line in running.lines() {
        let mut fields = line.split_whitespace();
        let Some(unit) = fields.next() else { continue };
        let _load = fields.next();
        let _active = fields.next();
        let _sub = fields.next();
        let description = fields.collect::<Vec<_>>().join(" ");
        if is_application_service(unit) {
            visible_count += 1;
            lines.push(format!("- `{unit}` — {description}"));
        }
    }
    if visible_count == 0 {
        lines.push("- 无".to_owned());
    }

    lines.push(String::new());
    lines.push("### 应用定时任务".to_owned());
    let timers = command_output(
        "systemctl",
        &["list-timers", "--all", "--no-legend", "--plain"],
    )
    .unwrap_or_default();
    let mut timer_units = HashSet::new();
    for line in timers.lines() {
        for field in line.split_whitespace() {
            if field.ends_with(".timer") && is_application_timer(field) {
                timer_units.insert(field.to_owned());
            }
        }
    }
    let mut timer_units = timer_units.into_iter().collect::<Vec<_>>();
    timer_units.sort();
    if timer_units.is_empty() {
        lines.push("- 无".to_owned());
    } else {
        lines.extend(timer_units.into_iter().map(|unit| format!("- `{unit}`")));
    }

    lines.push(String::new());
    lines.push("### Docker 容器".to_owned());
    let containers = command_output(
        "docker",
        &["ps", "--format", "{{.Names}} | {{.Image}} | {{.Status}}"],
    )
    .unwrap_or_default();
    if containers.trim().is_empty() {
        lines.push("- 无运行中的容器".to_owned());
    } else {
        lines.extend(containers.lines().map(|line| format!("- `{line}`")));
    }
    lines.join("\n")
}

fn is_application_service(unit: &str) -> bool {
    const MARKERS: &[&str] = &[
        "term-pigeon",
        "nginx",
        "xray",
        "docker",
        "containerd",
        "podman",
        "caddy",
        "httpd",
        "apache",
        "postgres",
        "mysql",
        "mariadb",
        "redis",
        "ollama",
        "telegram",
        "fail2ban",
    ];
    MARKERS.iter().any(|marker| unit.contains(marker))
}

fn is_application_timer(unit: &str) -> bool {
    unit.starts_with("oci-") || unit.contains("telegram") || unit.contains("term-pigeon")
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn memory_summary(meminfo: &str) -> Option<String> {
    let mut total_kib = None;
    let mut available_kib = None;
    for line in meminfo.lines() {
        let mut fields = line.split_whitespace();
        match fields.next()? {
            "MemTotal:" => total_kib = fields.next()?.parse::<u64>().ok(),
            "MemAvailable:" => available_kib = fields.next()?.parse::<u64>().ok(),
            _ => {}
        }
    }
    let total = total_kib?;
    let available = available_kib?;
    let used = total.saturating_sub(available);
    Some(format!("{:.1} GiB / {:.1} GiB 已用", gib(used), gib(total)))
}

fn gib(kib: u64) -> f64 {
    kib as f64 / 1024.0 / 1024.0
}

fn format_uptime(seconds: f64) -> String {
    let minutes = (seconds as u64) / 60;
    let days = minutes / 1_440;
    let hours = (minutes % 1_440) / 60;
    let minutes = minutes % 60;
    if days > 0 {
        format!("{days} 天 {hours} 小时 {minutes} 分")
    } else {
        format!("{hours} 小时 {minutes} 分")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_memory() {
        let value = "MemTotal:       4194304 kB\nMemFree: 1 kB\nMemAvailable:   3145728 kB\n";
        assert_eq!(
            memory_summary(value).as_deref(),
            Some("1.0 GiB / 4.0 GiB 已用")
        );
    }

    #[test]
    fn formats_uptime() {
        assert_eq!(format_uptime(183_660.0), "2 天 3 小时 1 分");
    }
}
