from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest


CRATE_ROOT = Path(__file__).resolve().parents[1]


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--profile",
        action="store",
        choices=("smoke", "extensive", "stress"),
        default="smoke",
        help="amount of randomized and exhaustive differential testing",
    )


@pytest.fixture(scope="session")
def profile(pytestconfig: pytest.Config) -> str:
    return str(pytestconfig.getoption("--profile"))


@pytest.fixture(scope="session")
def randomized_case_count(profile: str) -> int:
    return {"smoke": 8, "extensive": 40, "stress": 250}[profile]


@pytest.fixture(scope="session")
def cli_path() -> Path:
    configured = os.environ.get("SUPPORTED_AP_CLI")
    if configured:
        binary = Path(configured).expanduser().resolve()
        if not binary.is_file():
            pytest.fail(f"SUPPORTED_AP_CLI does not name a file: {binary}")
        return binary

    subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "supported_ap_metrics"],
        cwd=CRATE_ROOT,
        check=True,
    )
    binary = CRATE_ROOT / "target" / "debug" / "supported_ap_metrics"
    if not binary.is_file():
        pytest.fail(f"cargo did not produce the expected CLI: {binary}")
    return binary
