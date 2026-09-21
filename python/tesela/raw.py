"""Advanced engine/interchange values."""
from typing import Any, Mapping
from ._native import normalize_spec


def schema(document: Mapping[str, Any]) -> dict[str, Any]:
    """Validate and normalize a raw semantic schema document."""
    return normalize_spec(dict(document))
