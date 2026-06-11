"""Hand-written runtime for the FinX Platform Python SDK.

This module is **not** generated: it is the small, stable core the generated
namespace modules call into. It has zero third-party dependencies — only the
Python standard library (``urllib.request``) — so ``pip install finx-platform``
pulls nothing transitive. Optional dataframe conversion lazily imports pandas /
polars only when you ask for it.

Design:

* :class:`Client` builds ``GET /api/v1/<route>?<query>`` requests against a local
  daemon's REST surface and returns a :class:`FinXObject`. The base URL comes
  from the ``base_url`` argument, then the ``FINX_BASE_URL`` environment
  variable, then the loopback default (the daemon's default REST bind).
* The transport is injectable: pass ``opener=`` (anything with the
  ``urlopen(request, timeout=...)`` shape) to make the client testable without a
  network — the test-suite stubs it.
* HTTP errors are mapped to Python exceptions: ``400`` → :class:`ValueError`
  (bad route / invalid query), ``502`` → :class:`RuntimeError` (every provider
  failed), other non-2xx → :class:`RuntimeError`.
* :class:`FinXObject` wraps the standardized result envelope
  (``id`` / ``results`` / ``provider`` / ``warnings`` / ``extra``) and offers
  ``to_dict`` (always), ``to_dataframe`` (pandas) and ``to_polars`` (polars).
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Mapping

#: REST base URL used when neither the constructor nor ``FINX_BASE_URL`` provides
#: one — the daemon's default ``TDW_DAEMON_REST_BIND`` loopback bind.
DEFAULT_BASE_URL = "http://127.0.0.1:7879"

#: Environment variable that overrides the default base URL.
BASE_URL_ENV = "FINX_BASE_URL"

#: The route-family prefix every catalog route is served under.
API_PREFIX = "/api/v1/"

# An opener is anything exposing ``urlopen(request, timeout=...) -> response``;
# ``urllib.request.urlopen`` is the default, and tests inject a stub.
Opener = Callable[..., Any]


class FinXObject:
    """A standardized result envelope returned by a catalog route.

    Wraps the daemon's ``ResultEnvelope`` JSON: ``id`` (the route), ``results``
    (the record list), ``provider`` (the key that served the request),
    ``warnings`` (non-fatal advisories), and ``extra`` (route / arguments
    provenance, plus any server-rendered ``chart`` payload).
    """

    __slots__ = ("_raw",)

    def __init__(self, raw: Mapping[str, Any]) -> None:
        self._raw = dict(raw)

    @property
    def results(self) -> list[Any]:
        """The standardized records (possibly empty)."""
        value = self._raw.get("results")
        return list(value) if isinstance(value, list) else []

    @property
    def provider(self) -> str | None:
        """The provider key that served the request, if reported."""
        value = self._raw.get("provider")
        return value if isinstance(value, str) else None

    @property
    def warnings(self) -> list[Any]:
        """Non-fatal warnings accumulated while producing the result."""
        value = self._raw.get("warnings")
        return list(value) if isinstance(value, list) else []

    @property
    def extra(self) -> dict[str, Any]:
        """Route / timestamp / arguments provenance (and any ``chart`` payload)."""
        value = self._raw.get("extra")
        return dict(value) if isinstance(value, dict) else {}

    @property
    def chart(self) -> Any | None:
        """The server-rendered chart payload, if the request asked for one.

        The REST envelope carries any chart under ``extra``; this is a
        convenience accessor that returns ``None`` when absent.
        """
        chart = self.extra.get("chart")
        if chart is not None:
            return chart
        return self._raw.get("chart")

    def to_dict(self) -> dict[str, Any]:
        """The raw envelope as a plain ``dict`` (always available, no deps)."""
        return dict(self._raw)

    def to_dataframe(self) -> Any:
        """Convert ``results`` to a :class:`pandas.DataFrame`.

        Raises:
            ImportError: If pandas is not installed (``pip install
                finx-platform[pandas]``).
        """
        try:
            import pandas  # noqa: PLC0415  (lazy import: optional dependency)
        except ImportError as exc:  # pragma: no cover - exercised via injection
            raise ImportError(
                "to_dataframe() requires pandas; install it with "
                "`pip install finx-platform[pandas]` or `pip install pandas`"
            ) from exc
        return pandas.DataFrame(self.results)

    def to_polars(self) -> Any:
        """Convert ``results`` to a :class:`polars.DataFrame`.

        Raises:
            ImportError: If polars is not installed (``pip install
                finx-platform[polars]``).
        """
        try:
            import polars  # noqa: PLC0415  (lazy import: optional dependency)
        except ImportError as exc:  # pragma: no cover - exercised via injection
            raise ImportError(
                "to_polars() requires polars; install it with "
                "`pip install finx-platform[polars]` or `pip install polars`"
            ) from exc
        return polars.DataFrame(self.results)

    def __repr__(self) -> str:
        return (
            f"FinXObject(id={self._raw.get('id')!r}, provider={self.provider!r}, "
            f"results={len(self.results)})"
        )


class Client:
    """A minimal REST client for the daemon's catalog ``/api/v1`` surface.

    Args:
        base_url: REST base URL. Falls back to ``FINX_BASE_URL`` then the
            loopback default. A trailing slash is normalized away.
        timeout: Per-request timeout in seconds.
        opener: Transport hook with the ``urlopen(request, timeout=...)`` shape;
            defaults to :func:`urllib.request.urlopen`. Injectable for testing.
    """

    def __init__(
        self,
        base_url: str | None = None,
        *,
        timeout: float = 30.0,
        opener: Opener | None = None,
    ) -> None:
        resolved = base_url or os.environ.get(BASE_URL_ENV) or DEFAULT_BASE_URL
        self.base_url = resolved.rstrip("/")
        self.timeout = timeout
        self._opener: Opener = opener if opener is not None else urllib.request.urlopen

    def build_url(self, route: str, params: Mapping[str, Any]) -> str:
        """Build the full request URL for ``route`` and ``params``.

        Booleans serialize as ``true`` / ``false``; ``None`` values are dropped;
        everything else is stringified. The route is appended under
        ``/api/v1/`` verbatim (catalog routes are already URL-safe slugs).
        """
        query = urllib.parse.urlencode(_encode_params(params))
        url = f"{self.base_url}{API_PREFIX}{route}"
        return f"{url}?{query}" if query else url

    def fetch(self, route: str, params: Mapping[str, Any]) -> FinXObject:
        """Issue ``GET /api/v1/<route>`` and wrap the envelope in a FinXObject.

        Raises:
            ValueError: On a ``400`` response (unknown route / invalid query),
                carrying the server's error message.
            RuntimeError: On a ``502`` (every provider failed) or any other
                non-success response, carrying the server's error message.
        """
        url = self.build_url(route, params)
        request = urllib.request.Request(url, method="GET")  # noqa: S310 (loopback http by design)
        try:
            with self._opener(request, timeout=self.timeout) as response:
                payload = response.read()
        except urllib.error.HTTPError as exc:
            raise _map_http_error(exc) from exc
        return FinXObject(_decode_envelope(payload))


def _encode_params(params: Mapping[str, Any]) -> list[tuple[str, str]]:
    """Normalize a params mapping into urlencode-ready ``(key, value)`` pairs.

    Drops ``None`` values, renders bools as lowercase ``true`` / ``false``, and
    stringifies the rest. Returns a sorted list so URLs are deterministic.
    """
    pairs: list[tuple[str, str]] = []
    for key, value in params.items():
        if value is None:
            continue
        if isinstance(value, bool):
            pairs.append((key, "true" if value else "false"))
        else:
            pairs.append((key, str(value)))
    pairs.sort()
    return pairs


def _decode_envelope(payload: bytes) -> dict[str, Any]:
    """Decode a response body into the envelope mapping.

    Raises:
        RuntimeError: If the body is not a JSON object.
    """
    try:
        decoded = json.loads(payload.decode("utf-8"))
    except (ValueError, UnicodeDecodeError) as exc:
        raise RuntimeError(f"daemon returned a non-JSON response: {exc}") from exc
    if not isinstance(decoded, dict):
        raise RuntimeError("daemon returned a non-object JSON response")
    return decoded


def _map_http_error(exc: urllib.error.HTTPError) -> Exception:
    """Map an HTTP error status to the SDK's exception type.

    ``400`` → :class:`ValueError`; everything else (``404`` / ``405`` / ``502`` /
    ``5xx``) → :class:`RuntimeError`. The server's error message is extracted
    from the JSON body when present.
    """
    message = _error_message(exc)
    status = exc.code
    if status == 400:
        return ValueError(f"invalid request (400): {message}")
    if status == 502:
        return RuntimeError(f"all providers failed (502): {message}")
    return RuntimeError(f"request failed ({status}): {message}")


def _error_message(exc: urllib.error.HTTPError) -> str:
    """Extract the server's error string from an HTTPError body, if any."""
    try:
        body = exc.read()
    except (OSError, ValueError):
        return exc.reason if isinstance(exc.reason, str) else str(exc.reason)
    try:
        decoded = json.loads(body.decode("utf-8"))
    except (ValueError, UnicodeDecodeError):
        text = body.decode("utf-8", errors="replace").strip()
        return text or (exc.reason if isinstance(exc.reason, str) else str(exc.reason))
    if isinstance(decoded, dict) and isinstance(decoded.get("error"), str):
        return decoded["error"]
    return json.dumps(decoded)
