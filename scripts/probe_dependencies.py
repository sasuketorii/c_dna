"""Record registry observations without installing or executing package contents."""
from __future__ import annotations
import json
import platform
import urllib.request
from pathlib import Path


def read_json(url: str) -> dict:
    request = urllib.request.Request(url, headers={"User-Agent": "C-DNA-dependency-audit/0.1"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def main() -> None:
    report: dict = {"python": platform.python_version(), "system": platform.platform(), "npm": {}, "pypi": {}, "crates": {}}
    for name in ["react", "react-dom", "vite", "typescript", "pnpm", "@vitejs/plugin-react", "@tauri-apps/api", "@tauri-apps/cli", "@tanstack/react-query", "@anthropic-ai/claude-agent-sdk", "@playwright/test", "lucide-react", "zod", "tailwindcss", "@tailwindcss/vite"]:
        try:
            info = read_json(f"https://registry.npmjs.org/{name}/latest")
            report["npm"][name] = {"version": info["version"], "license": info.get("license"), "engines": info.get("engines"), "integrity": info.get("dist", {}).get("integrity")}
        except Exception as exc:
            report["npm"][name] = {"error_type": type(exc).__name__}
    for name in ["numpy", "scipy", "scikit-learn", "lightgbm", "polars", "pydantic", "pytest", "jsonschema", "uv"]:
        try:
            info = read_json(f"https://pypi.org/pypi/{name}/json")["info"]
            report["pypi"][name] = {"version": info["version"], "requires_python": info.get("requires_python")}
        except Exception as exc:
            report["pypi"][name] = {"error_type": type(exc).__name__}
    for name in ["rusqlite", "rmcp", "tauri", "tauri-build", "serde", "serde_json", "uuid", "tokio", "thiserror", "sha2", "zeroize", "argon2", "chacha20poly1305", "keyring", "tempfile", "schemars", "zip", "jsonschema"]:
        try:
            info = read_json(f"https://crates.io/api/v1/crates/{name}")["crate"]
            report["crates"][name] = {"version": info["max_stable_version"]}
        except Exception as exc:
            report["crates"][name] = {"error_type": type(exc).__name__}
    Path("verification").mkdir(exist_ok=True)
    Path("verification/registry-observations.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
