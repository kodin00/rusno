use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use bollard::container::ListContainersOptions;
use bollard::container::PruneContainersOptions;
use bollard::image::PruneImagesOptions;
use bollard::models::ContainerSummary;
use bollard::Docker;
use sysinfo::{Disks, System};

/// Wrapper around the Docker Engine API (bollard) + sysinfo for host telemetry.
#[derive(Clone)]
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    pub async fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| anyhow::anyhow!("failed to connect to Docker: {e}"))?;
        // Ping to verify connection
        docker
            .ping()
            .await
            .context("Docker daemon not reachable — is docker running?")?;
        Ok(Self { docker })
    }

    /// List running containers managed by rusno (labeled `rusno.project=<slug>`).
    pub async fn list_rusno_containers(&self) -> Result<Vec<ContainerSummary>> {
        let opts = ListContainersOptions {
            all: false,
            filters: HashMap::from([("label".to_string(), vec!["rusno.project".to_string()])]),
            ..Default::default()
        };
        let containers = self.docker.list_containers(Some(opts)).await?;
        Ok(containers)
    }

    /// List all running containers on the host (no label filter). Used by the
    /// sidebar so the user can see *every* running container, not just the ones
    /// rusno started. rusno-managed ones are still distinguishable via their
    /// `rusno.project` label (see `templates::sidebar::container_row`).
    pub async fn list_running_containers(&self) -> Result<Vec<ContainerSummary>> {
        let opts = ListContainersOptions::<String> {
            all: false,
            ..Default::default()
        };
        let containers = self.docker.list_containers(Some(opts)).await?;
        Ok(containers)
    }

    /// Count running rusno-managed containers.
    pub async fn count_rusno_containers(&self) -> Result<usize> {
        Ok(self.list_rusno_containers().await?.len())
    }

    /// Get docker system disk usage summary.
    pub async fn system_df(&self) -> Result<DiskUsageSummary> {
        let df = self.docker.df().await?;
        let total_size = df
            .images
            .as_ref()
            .map(|imgs| imgs.iter().map(|i| i.size).sum::<i64>())
            .unwrap_or(0);
        Ok(DiskUsageSummary {
            images: df.images.unwrap_or_default().len(),
            containers: df.containers.unwrap_or_default().len(),
            volumes: df.volumes.unwrap_or_default().len(),
            build_cache_count: df.build_cache.as_ref().map(|c| c.len()).unwrap_or(0),
            total_size,
        })
    }

    /// Safe prune: unused images + build cache. Running containers are not affected.
    pub async fn prune_safe(&self) -> Result<String> {
        let mut output = String::new();

        match self
            .docker
            .prune_images(Some(PruneImagesOptions::<String> {
                ..Default::default()
            }))
            .await
        {
            Ok(result) => {
                let reclaimed = result.space_reclaimed.unwrap_or(0);
                output.push_str(&format!(
                    "Pruned images, reclaimed {:.1} MB\n",
                    reclaimed as f64 / 1_048_576.0
                ));
            }
            Err(e) => {
                output.push_str(&format!("Image prune error: {e}\n"));
            }
        }

        // Build cache prune via system prune without volumes
        match self
            .docker
            .prune_containers(Some(PruneContainersOptions::<String> {
                ..Default::default()
            }))
            .await
        {
            Ok(result) => {
                let reclaimed = result.space_reclaimed.unwrap_or(0);
                output.push_str(&format!(
                    "Pruned stopped containers, reclaimed {:.1} MB\n",
                    reclaimed as f64 / 1_048_576.0
                ));
            }
            Err(e) => {
                output.push_str(&format!("Container prune error: {e}\n"));
            }
        }

        Ok(output)
    }

    /// Nuclear prune: system prune -a --volumes. Removes everything unused including volumes.
    pub async fn prune_nuclear(&self) -> Result<String> {
        // We shell out for this since bollard's prune APIs are granular
        let output = tokio::process::Command::new("docker")
            .args(["system", "prune", "-a", "--volumes", "-f"])
            .output()
            .await
            .context("failed to run docker system prune")?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            anyhow::bail!("docker system prune failed: {stderr}");
        }
        Ok(stdout.to_string())
    }

    /// Run `docker compose -f <compose_path> <command>` in the project folder.
    pub async fn compose_up(
        &self,
        folder_path: &Path,
        compose_path: &str,
        command: &str,
    ) -> Result<std::process::Output> {
        let compose_file = folder_path.join(compose_path);
        let compose_file_str = compose_file.to_string_lossy();

        // Split command into args (e.g. "up -d --build --remove-orphans")
        let args: Vec<&str> = command.split_whitespace().collect();

        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(["compose", "-f", &compose_file_str]);
        cmd.args(&args);
        cmd.current_dir(folder_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let output = cmd.output().await.context("failed to run docker compose")?;
        Ok(output)
    }

    /// Run `docker compose -f <path> stop`.
    pub async fn compose_stop(
        &self,
        folder_path: &Path,
        compose_path: &str,
    ) -> Result<std::process::Output> {
        let compose_file = folder_path.join(compose_path);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(["compose", "-f", &compose_file.to_string_lossy(), "stop"]);
        cmd.current_dir(folder_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        Ok(cmd.output().await?)
    }

    /// Run `docker compose -f <path> restart`.
    pub async fn compose_restart(
        &self,
        folder_path: &Path,
        compose_path: &str,
    ) -> Result<std::process::Output> {
        let compose_file = folder_path.join(compose_path);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(["compose", "-f", &compose_file.to_string_lossy(), "restart"]);
        cmd.current_dir(folder_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        Ok(cmd.output().await?)
    }

    /// Run `docker compose -f <path> down -v --remove-orphans`.
    pub async fn compose_down(
        &self,
        folder_path: &Path,
        compose_path: &str,
    ) -> Result<std::process::Output> {
        let compose_file = folder_path.join(compose_path);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args([
            "compose",
            "-f",
            &compose_file.to_string_lossy(),
            "down",
            "-v",
            "--remove-orphans",
        ]);
        cmd.current_dir(folder_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        Ok(cmd.output().await?)
    }

    /// Poll `docker compose ps` to check container health.
    /// Returns true if all services are running (or healthy if healthchecks defined).
    pub async fn compose_ps_healthy(&self, folder_path: &Path, compose_path: &str) -> Result<bool> {
        let compose_file = folder_path.join(compose_path);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args([
            "compose",
            "-f",
            &compose_file.to_string_lossy(),
            "ps",
            "--format",
            "json",
        ]);
        cmd.current_dir(folder_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let output = cmd
            .output()
            .await
            .context("failed to run docker compose ps")?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Each line is a JSON object. Parse and check status.
        for line in stdout.lines() {
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                let state = v.get("State").and_then(|s| s.as_str()).unwrap_or("");
                let health = v.get("Health").and_then(|s| s.as_str());
                // If the service has a healthcheck, require "healthy"
                if let Some(h) = health {
                    if h != "healthy" {
                        return Ok(false);
                    }
                } else {
                    // No healthcheck — require "running"
                    if state != "running" {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }

    /// Get `docker compose logs --tail=200` for failure diagnostics.
    pub async fn compose_logs_tail(
        &self,
        folder_path: &Path,
        compose_path: &str,
        tail: usize,
    ) -> Result<String> {
        let compose_file = folder_path.join(compose_path);
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args([
            "compose",
            "-f",
            &compose_file.to_string_lossy(),
            "logs",
            "--tail",
            &tail.to_string(),
        ]);
        cmd.current_dir(folder_path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let output = cmd.output().await?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

/// Summary of docker disk usage for the Settings page.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiskUsageSummary {
    pub images: usize,
    pub containers: usize,
    pub volumes: usize,
    pub build_cache_count: usize,
    pub total_size: i64,
}

// ============================================================
// Host telemetry via sysinfo
// ============================================================

/// Collect host telemetry: CPU, memory, and storage for the disk holding projects_root.
pub struct Telemetry {
    sys: System,
}

impl Telemetry {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Self { sys }
    }

    /// Refresh and return current telemetry snapshot.
    pub fn snapshot(&mut self, projects_root: &Path) -> TelemetrySnapshot {
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();

        let cpu_usage = self.sys.global_cpu_usage();

        let total_mem = self.sys.total_memory();
        let used_mem = self.sys.used_memory();
        let mem_percent = if total_mem > 0 {
            (used_mem as f64 / total_mem as f64 * 100.0) as u32
        } else {
            0
        };

        // Find the disk that contains projects_root
        let disks = Disks::new_with_refreshed_list();
        let storage = find_disk_for_path(&disks, projects_root);

        TelemetrySnapshot {
            cpu_percent: cpu_usage.round() as u32,
            mem_used: used_mem,
            mem_total: total_mem,
            mem_percent,
            storage_used: storage.used,
            storage_total: storage.total,
            storage_percent: storage.percent,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TelemetrySnapshot {
    pub cpu_percent: u32,
    pub mem_used: u64,
    pub mem_total: u64,
    pub mem_percent: u32,
    pub storage_used: u64,
    pub storage_total: u64,
    pub storage_percent: u32,
}

struct DiskInfo {
    used: u64,
    total: u64,
    percent: u32,
}

fn find_disk_for_path(disks: &Disks, path: &Path) -> DiskInfo {
    // Find the mount point that is the longest prefix of `path`
    let path_str = path.to_string_lossy();
    let mut best: Option<&sysinfo::Disk> = None;
    let mut best_len = 0;

    for disk in disks.list() {
        let mount = disk.mount_point();
        let mount_str = mount.to_string_lossy();
        if path_str.starts_with(mount_str.as_ref()) && mount_str.len() > best_len {
            best = Some(disk);
            best_len = mount_str.len();
        }
    }

    if let Some(disk) = best {
        let total = disk.total_space();
        let free = disk.available_space();
        let used = total.saturating_sub(free);
        let percent = if total > 0 {
            (used as f64 / total as f64 * 100.0) as u32
        } else {
            0
        };
        DiskInfo {
            used,
            total,
            percent,
        }
    } else {
        DiskInfo {
            used: 0,
            total: 0,
            percent: 0,
        }
    }
}

/// Format bytes as human-readable (e.g. "4.2 GB").
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
