"""Stdlib ``unittest`` suite for the FinX Platform SDK runtime.

No third-party dependencies (no pytest): run with

    python3 -m unittest discover sdk/python/tests

The HTTP transport is injected as a stub ``opener`` so every test runs without a
network. The stub mimics ``urllib.request.urlopen``'s context-manager + read
shape, and an error stub raises ``urllib.error.HTTPError`` to exercise mapping.
"""

from __future__ import annotations

import io
import json
import os
import sys
import unittest
import urllib.error
from pathlib import Path
from typing import Any

# Make the package importable without installation (CI may not `pip install`).
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from finx_platform import FinX, FinXObject  # noqa: E402
from finx_platform._client import (  # noqa: E402
    DEFAULT_BASE_URL,
    Client,
    _encode_params,
)


class _StubResponse(io.BytesIO):
    """A minimal context-manager response wrapping a JSON body."""

    def __enter__(self) -> "_StubResponse":
        return self

    def __exit__(self, *exc: object) -> bool:
        self.close()
        return False


class _RecordingOpener:
    """A stub ``urlopen`` that records the request and returns a fixed body."""

    def __init__(self, body: Any) -> None:
        self.body = body
        self.last_url: str | None = None
        self.last_timeout: float | None = None

    def __call__(self, request: Any, timeout: float | None = None) -> _StubResponse:
        self.last_url = request.full_url
        self.last_timeout = timeout
        return _StubResponse(json.dumps(self.body).encode("utf-8"))


def _http_error(status: int, body: dict[str, Any]) -> urllib.error.HTTPError:
    payload = io.BytesIO(json.dumps(body).encode("utf-8"))
    return urllib.error.HTTPError(
        url="http://127.0.0.1:7879/api/v1/x",
        code=status,
        msg="error",
        hdrs=None,  # type: ignore[arg-type]
        fp=payload,
    )


class _ErrorOpener:
    """A stub ``urlopen`` that raises a fixed ``HTTPError``."""

    def __init__(self, error: urllib.error.HTTPError) -> None:
        self.error = error

    def __call__(self, request: Any, timeout: float | None = None) -> Any:
        raise self.error


ENVELOPE = {
    "id": "equity/price/historical",
    "results": [
        {"symbol": "AAPL", "date": "2026-05-20", "close": 101.0},
        {"symbol": "AAPL", "date": "2026-05-21", "close": 102.0},
    ],
    "provider": "fileset",
    "warnings": [],
    "extra": {"route": "equity/price/historical", "arguments": {"provider": "fileset"}},
}


class UrlBuildingTests(unittest.TestCase):
    def test_base_url_default_and_route_prefix(self) -> None:
        client = Client(opener=_RecordingOpener(ENVELOPE))
        url = client.build_url("equity/price/historical", {})
        self.assertEqual(url, f"{DEFAULT_BASE_URL}/api/v1/equity/price/historical")

    def test_constructor_base_url_overrides_and_strips_slash(self) -> None:
        client = Client("http://example.test:9000/", opener=_RecordingOpener(ENVELOPE))
        url = client.build_url("economy/cpi", {})
        self.assertEqual(url, "http://example.test:9000/api/v1/economy/cpi")

    def test_env_base_url_is_used_when_no_arg(self) -> None:
        os.environ["FINX_BASE_URL"] = "http://env.test:1234"
        try:
            client = Client(opener=_RecordingOpener(ENVELOPE))
            self.assertEqual(client.base_url, "http://env.test:1234")
        finally:
            del os.environ["FINX_BASE_URL"]

    def test_query_is_sorted_and_deterministic(self) -> None:
        client = Client(opener=_RecordingOpener(ENVELOPE))
        url = client.build_url(
            "equity/price/historical", {"symbol": "AAPL", "limit": 10, "provider": "yahoo"}
        )
        self.assertEqual(
            url,
            f"{DEFAULT_BASE_URL}/api/v1/equity/price/historical"
            "?limit=10&provider=yahoo&symbol=AAPL",
        )


class ParamEncodingTests(unittest.TestCase):
    def test_none_values_are_dropped(self) -> None:
        self.assertEqual(_encode_params({"a": None, "b": 1}), [("b", "1")])

    def test_bools_render_lowercase(self) -> None:
        self.assertEqual(
            _encode_params({"chart": True, "x": False}),
            [("chart", "true"), ("x", "false")],
        )

    def test_values_are_stringified_and_sorted(self) -> None:
        self.assertEqual(
            _encode_params({"z": 2, "a": "AAPL"}), [("a", "AAPL"), ("z", "2")]
        )


class KwargMappingTests(unittest.TestCase):
    def test_namespace_method_threads_kwargs_and_typed_params(self) -> None:
        opener = _RecordingOpener(ENVELOPE)
        finx = FinX(client=Client(opener=opener))
        finx.equity.price.historical(symbol="AAPL", provider="yahoo", limit=5)
        assert opener.last_url is not None
        self.assertIn("/api/v1/equity/price/historical?", opener.last_url)
        self.assertIn("symbol=AAPL", opener.last_url)
        self.assertIn("provider=yahoo", opener.last_url)
        self.assertIn("limit=5", opener.last_url)

    def test_chartable_route_emits_chart_true(self) -> None:
        opener = _RecordingOpener(ENVELOPE)
        finx = FinX(client=Client(opener=opener))
        # equity/price/historical is chartable.
        finx.equity.price.historical(symbol="AAPL", chart=True)
        assert opener.last_url is not None
        self.assertIn("chart=true", opener.last_url)

    def test_compute_route_raises_not_implemented(self) -> None:
        finx = FinX(client=Client(opener=_RecordingOpener(ENVELOPE)))
        with self.assertRaises(NotImplementedError):
            finx.technical.rsi(length=14)


class FinXObjectTests(unittest.TestCase):
    def test_accessors_and_to_dict(self) -> None:
        obj = FinXObject(ENVELOPE)
        self.assertEqual(obj.provider, "fileset")
        self.assertEqual(len(obj.results), 2)
        self.assertEqual(obj.warnings, [])
        self.assertEqual(obj.extra["route"], "equity/price/historical")
        self.assertEqual(obj.to_dict()["id"], "equity/price/historical")

    def test_chart_accessor_reads_extra_then_top_level(self) -> None:
        self.assertIsNone(FinXObject(ENVELOPE).chart)
        with_extra = dict(ENVELOPE, extra={"chart": {"kind": "line"}})
        self.assertEqual(FinXObject(with_extra).chart, {"kind": "line"})
        with_top = {"id": "x", "results": [], "chart": {"kind": "bar"}}
        self.assertEqual(FinXObject(with_top).chart, {"kind": "bar"})

    def test_to_dataframe_without_pandas_raises_importerror(self) -> None:
        obj = FinXObject(ENVELOPE)
        try:
            import pandas  # noqa: F401
        except ImportError:
            with self.assertRaises(ImportError):
                obj.to_dataframe()
        else:
            frame = obj.to_dataframe()
            self.assertEqual(len(frame), 2)

    def test_fetch_returns_finxobject(self) -> None:
        client = Client(opener=_RecordingOpener(ENVELOPE))
        obj = client.fetch("equity/price/historical", {"symbol": "AAPL"})
        self.assertIsInstance(obj, FinXObject)
        self.assertEqual(obj.provider, "fileset")


class ErrorMappingTests(unittest.TestCase):
    def test_400_maps_to_value_error_with_server_message(self) -> None:
        client = Client(
            opener=_ErrorOpener(_http_error(400, {"error": "unknown catalog route: x"}))
        )
        with self.assertRaises(ValueError) as ctx:
            client.fetch("x", {})
        self.assertIn("unknown catalog route", str(ctx.exception))

    def test_502_maps_to_runtime_error(self) -> None:
        client = Client(
            opener=_ErrorOpener(_http_error(502, {"error": "all providers failed"}))
        )
        with self.assertRaises(RuntimeError) as ctx:
            client.fetch("equity/price/historical", {})
        self.assertIn("502", str(ctx.exception))

    def test_other_status_maps_to_runtime_error(self) -> None:
        client = Client(opener=_ErrorOpener(_http_error(404, {"error": "not found"})))
        with self.assertRaises(RuntimeError):
            client.fetch("nope", {})


if __name__ == "__main__":
    unittest.main()
