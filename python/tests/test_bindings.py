from __future__ import annotations

import datetime
import decimal
import enum
import uuid
from dataclasses import dataclass
from typing import Annotated

import pytest

from tesela import MemoryBackend, Runtime, Spec, TeselaError, Vector


def ontology():
    spec = Spec()

    class Tier(enum.Enum):
        STANDARD = "standard"
        PREMIUM = "premium"

    @spec.object_type(primary_key="id")
    @dataclass
    class Customer:
        id: uuid.UUID
        joined: datetime.date
        seen_at: datetime.datetime
        balance: decimal.Decimal
        tier: Tier
        embedding: Annotated[list[float], Vector(3)]
        email: str | None

    @spec.link(from_type=Customer, to_type=Customer)
    class Referral:
        pass

    @spec.action(subject=Customer, effects={"reads": True, "writes": False, "external": False, "idempotent": True}, approval="never")
    class LookupCustomer:
        id: uuid.UUID

    return spec, Customer, Referral, LookupCustomer, Tier


def context():
    return {"principal": {"id": "test-agent", "claims": {"tenant": "acme"}}, "request_id": "test-request"}


def test_semantic_spec_round_trip_and_action_classes_only():
    spec, Customer, _, _, _ = ontology()
    document = spec.to_dict()
    assert set(document) == {"version", "object_types", "link_types", "actions"}
    assert Spec.from_json(spec.to_json()).to_dict() == document
    assert Customer.id.name == "id"
    with pytest.raises(TypeError, match="declaration class"):
        spec.action(subject=Customer)(lambda: None)


def test_typed_runtime_and_direct_scalar_conversion():
    spec, Customer, _, _, Tier = ontology()
    backend = MemoryBackend()
    runtime = Runtime(spec, objects={Customer: backend}, policy="allow_all")
    identifier = uuid.uuid4()
    runtime.mutate(Customer, {"create": {"values": {
        "id": identifier,
        "joined": datetime.date(2026, 9, 20),
        "seen_at": datetime.datetime(2026, 9, 20, 10, 30, tzinfo=datetime.timezone.utc),
        "balance": decimal.Decimal("12.50"),
        "tier": Tier.PREMIUM,
        "embedding": [1.0, 2.0, 3.0],
        "email": "a@example.test",
    }}}, context=context())
    customer = runtime.get(Customer, identifier, context=context())
    assert isinstance(customer, Customer)
    assert customer.id == identifier
    assert customer.joined == datetime.date(2026, 9, 20)
    assert customer.seen_at.tzinfo is not None
    assert customer.balance == decimal.Decimal("12.50")
    assert customer.tier is Tier.PREMIUM


def test_typed_query_builder_is_discoverable():
    spec, Customer, _, _, Tier = ontology()
    runtime = Runtime(spec, objects={Customer: MemoryBackend()}, policy="allow_all")
    for index in range(3):
        runtime.mutate(Customer, {"create": {"values": {"id": uuid.uuid4(), "joined": "2026-09-20", "seen_at": "2026-09-20T00:00:00", "balance": "1", "tier": "standard", "embedding": [0, 0, 0], "email": f"{index}@example.test"}}}, context=context())
    rows = runtime.objects(Customer, context=context()).where(Customer.tier == Tier.STANDARD.value).take(2).load()
    assert len(rows) == 2
    assert all(isinstance(row, Customer) for row in rows)


def test_actions_are_bound_separately_from_declarations():
    spec, Customer, _, LookupCustomer, _ = ontology()
    runtime = Runtime(spec, objects={Customer: MemoryBackend()}, policy="allow_all", actions={LookupCustomer: lambda request: {"status": "success", "output": request["input"]}})
    identifier = uuid.uuid4()
    result = runtime.execute_action(LookupCustomer, {"id": identifier}, context=context())
    assert result["output"]["output"]["id"] == str(identifier)


class FilteringPolicy:
    def evaluate(self, request):
        return {"allow": True, "row_filter": {"op": "eq", "field": "email", "value": "visible@example.test"}, "redactions": ["balance"]}


def test_python_policy_reentry_filters_get_and_redacts():
    spec, Customer, _, _, _ = ontology()
    backend = MemoryBackend()
    runtime = Runtime(spec, objects={Customer: backend}, policy=FilteringPolicy())
    common = {"joined": "2026-09-20", "seen_at": "2026-09-20T00:00:00", "balance": "3", "tier": "standard", "embedding": [0, 0, 0]}
    visible, hidden = uuid.uuid4(), uuid.uuid4()
    seed = Runtime(spec, objects={Customer: backend}, policy="allow_all")
    seed.mutate(Customer, {"create": {"values": {"id": visible, "email": "visible@example.test", **common}}}, context=context())
    seed.mutate(Customer, {"create": {"values": {"id": hidden, "email": "hidden@example.test", **common}}}, context=context())
    assert runtime.get(Customer, visible, context=context()).email == "visible@example.test"
    with pytest.raises(TeselaError):
        runtime.get(Customer, hidden, context=context())


def test_invalid_runtime_limit_is_a_typed_error():
    spec, Customer, _, _, _ = ontology()
    with pytest.raises(TeselaError) as raised:
        Runtime(spec, objects={Customer: MemoryBackend()}, policy="allow_all", max_query_limit=0)
    assert raised.value.code == "validation"
    assert raised.value.retryable is False
    assert raised.value.side_effect == "not_started"
