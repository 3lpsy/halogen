# Packer template for the halogen-builder Tart macOS VM (Apple targets: iOS +
# mac desktop; built manually in clones, never in Linux CI). Base: a cirruslabs
# Xcode image; provisions rust/ios targets, just/dx/nextest/sccache, tailwind,
# and tailscaled (logged out). Build with ../tart-build-vm.sh; 0 = keep base setting.

packer {
  required_plugins {
    tart = {
      version = ">= 1.12.0"
      source  = "github.com/cirruslabs/tart"
    }
  }
}

variable "base_image" {
  type    = string
  default = "ghcr.io/cirruslabs/macos-tahoe-xcode:latest"
}

variable "vm_name" {
  type    = string
  default = "halogen-builder"
}

# 0 = inherit the base image's CPU/memory/disk config. Rust builds like more
# than the base default — override via build.sh env (TART_VM_CPU / TART_VM_MEMORY_GB),
# or later with `tart set halogen-builder --cpu N --memory M`.
variable "cpu_count" {
  type    = number
  default = 0
}

variable "memory_gb" {
  type    = number
  default = 0
}

# The xcode base images ship a 140 GB virtual (sparse) disk — plenty for the
# toolchain + a few build workspaces, so growing it is opt-in.
# Repo-read token for the cache pre-warm clone (provision-prewarm.sh). Empty →
# the pre-warm step skips and the image ships with cold caches.
variable "forgejo_token" {
  type      = string
  default   = ""
  sensitive = true
}
# Root domain of the in-cluster services (REQUIRED — no default, so the
# internal domain never lives in the repo). tart-build-vm.sh passes it from
# .env; provisioning persists it into the image's ~/.zshenv.
variable "services_root_domain" {
  type = string
}
variable "forgejo_host" {
  type    = string
  default = ""   # empty → provisioning derives git.<services_root_domain>
}
# Repo owner on the in-cluster Forgejo (REQUIRED — never hardcoded in the repo).
variable "forgejo_owner" {
  type = string
}
variable "forgejo_repo" {
  type    = string
  default = "halogen"
}

variable "disk_size_gb" {
  type    = number
  default = 0
}

source "tart-cli" "halogen" {
  vm_base_name = var.base_image
  vm_name      = var.vm_name
  cpu_count    = var.cpu_count > 0 ? var.cpu_count : null
  memory_gb    = var.memory_gb > 0 ? var.memory_gb : null
  disk_size_gb = var.disk_size_gb > 0 ? var.disk_size_gb : null
  headless     = true
  # cirruslabs images: user admin / password admin, SSH enabled, auto-login.
  ssh_username = "admin"
  ssh_password = "admin"
  ssh_timeout  = "300s"
}

build {
  sources = ["source.tart-cli.halogen"]

  provisioner "shell" {
    environment_vars = [
      "TART_FORGEJO_TOKEN=${var.forgejo_token}",
      "SERVICES_ROOT_DOMAIN=${var.services_root_domain}",
      "TART_FORGEJO_HOST=${var.forgejo_host}",
      "TART_FORGEJO_OWNER=${var.forgejo_owner}",
      "TART_FORGEJO_REPO=${var.forgejo_repo}",
    ]
    scripts = [
      "scripts/provision-rust.sh",
      "scripts/provision-tailscale.sh",
      "scripts/provision-prewarm.sh",
      "scripts/provision-verify.sh",
    ]
  }
}
