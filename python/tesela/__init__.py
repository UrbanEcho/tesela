"""Tesela's immutable typed public facade."""
from enum import Enum

from . import policy, raw, types
from ._native import TeselaError as Error
from .backend import backend
from .context import Budget, Cancellation, Context, budget, context
from .expr import AggregateExpr, Expr, Order, col, count, lit
from .model import LinkDefinition, Vector, action, field, link, object, trait, vector
from .plan import (Aggregate, AggregationResult, Batch, BatchResult, Create, Delete,
                   Execution, Get, Page, Query, Traverse, Update, Upsert, batch,
                   create, delete, get, query, traverse, update, upsert)
from .runtime import MemoryBackend, Runtime, Schema


class ErrorCode(str, Enum):
    VALIDATION = "validation"
    BAD_REQUEST = "bad_request"
    UNKNOWN_OBJECT_TYPE = "unknown_object_type"
    UNKNOWN_PROPERTY = "unknown_property"
    UNKNOWN_LINK = "unknown_link"
    UNKNOWN_ACTION = "unknown_action"
    NOT_FOUND = "not_found"
    CONFLICT = "conflict"
    STALE_REVISION = "stale_revision"
    STALE_SCHEMA = "stale_schema"
    UNAUTHORIZED = "unauthorized"
    POLICY_DENIED = "policy_denied"
    APPROVAL_REQUIRED = "approval_required"
    PRECONDITION_FAILED = "precondition_failed"
    UNSUPPORTED_CAPABILITY = "unsupported_capability"
    UNAVAILABLE = "unavailable"
    BUDGET_EXHAUSTED = "budget_exhausted"
    CANCELLED = "cancelled"
    TIMEOUT = "timeout"
    TRANSIENT = "transient"
    ADAPTER = "adapter"
    INTERNAL = "internal"


__all__ = [
    "Aggregate", "AggregateExpr", "AggregationResult", "Batch", "BatchResult", "Budget",
    "Cancellation", "Context", "Create", "Delete", "Error", "ErrorCode", "Execution", "Expr",
    "Get", "LinkDefinition", "MemoryBackend", "Order", "Page", "Query", "Runtime", "Schema",
    "Traverse", "Update", "Upsert",
    "Vector", "action", "backend", "batch", "budget", "col", "context",
    "count", "create", "delete", "field", "get", "link", "lit", "object", "policy",
    "query", "raw", "trait", "traverse", "types", "update", "upsert", "vector",
]
