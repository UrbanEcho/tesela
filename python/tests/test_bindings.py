from __future__ import annotations

from dataclasses import FrozenInstanceError
from typing import Annotated

import pytest
import tesela as ts


@ts.object(key="id")
class Customer:
    id: str
    email: str | None = None
    active: bool = True
    score: int = 0
    embedding: Annotated[list[float], ts.vector(3)] = ts.field(default_factory=lambda: [0.0, 0.0, 0.0])


Referrals = ts.link("referrals", Customer, Customer, cardinality="many")


@ts.action(returns=Customer, pure=True)
class EchoCustomer:
    id: str


def ctx(**claims: object) -> ts.Context:
    return ts.context("test-agent", request_id="request-1", claims=claims)


def runtime(backend: object | None = None, *, policy=ts.policy.allow_all,
            links=None, actions=None) -> ts.Runtime:
    return ts.Runtime({Customer: backend or ts.MemoryBackend()}, policy=policy,
                      links=links, actions=actions)


def test_objects_and_plans_are_frozen_semantic_values() -> None:
    customer = Customer("c-1")
    assert customer.email is None
    assert not isinstance(Customer.active, ts.Expr)  # fields are never query descriptors
    assert "id" not in Customer.__dict__
    with pytest.raises(FrozenInstanceError):
        customer.active = False  # type: ignore[misc]
    base = ts.query(Customer)
    filtered = base.where(ts.col("active") & (ts.col("email") != None)).limit(10)  # noqa: E711
    assert base.predicates == ()
    assert len(filtered.predicates) == 1
    with pytest.raises(TypeError, match="Tesela expressions"):
        bool(ts.col("active"))
    update = ts.update(Customer, "c-1", email="next@example.test")
    with pytest.raises(TypeError):
        update.changes["email"] = "mutated@example.test"  # type: ignore[index]

    claims = {"tenant": "acme"}
    context = ts.context("test-agent", request_id="immutable", claims=claims)
    claims["tenant"] = "changed"
    assert context.claims["tenant"] == "acme"


def test_crud_returns_typed_values_and_optional_get() -> None:
    rt, context = runtime(), ctx(tenant="acme")
    created = rt.run(ts.create(Customer("c-1", "a@example.test", score=7)), context)
    assert created == Customer("c-1", "a@example.test", score=7)
    assert rt.run(ts.get(Customer, "missing"), context) is None
    assert rt.run(ts.get(Customer, "c-1"), context) == created
    page = rt.run(ts.query(Customer).where(ts.col("email").ends_with("example.test")), context)
    assert page.items == (created,)
    changed = rt.run(ts.update(Customer, "c-1", email="b@example.test"), context)
    assert changed.email == "b@example.test"
    execution = rt.execute(ts.get(Customer, "c-1"), context)
    assert execution.value == changed
    assert execution.receipt["operation"] == "read"
    rt.run(ts.delete(Customer, "c-1"), context)
    assert rt.run(ts.get(Customer, "c-1"), context) is None


def test_execution_requires_explicit_context() -> None:
    with pytest.raises(TypeError):
        ts.context("test-agent")  # type: ignore[reportCallIssue]
    with pytest.raises(ValueError, match="request_id"):
        ts.context("test-agent", request_id="")
    with pytest.raises(TypeError, match="explicit"):
        runtime().run(ts.query(Customer), None)  # type: ignore[arg-type]


def test_repository_callbacks_receive_typed_values_and_context() -> None:
    stored: dict[str, Customer] = {}
    seen: list[str] = []

    def create(context: ts.Context, value: Customer) -> Customer:
        seen.append(context.claims["tenant"])
        assert context.request_id == "request-1"
        assert context.budget is not None and context.budget.calls == 4
        stored[value.id] = value
        return value

    def get(_context: ts.Context, key: str) -> Customer | None:
        return stored.get(key)

    repository = ts.backend(scan=lambda _context: stored.values(), create=create, get=get)
    rt = runtime(repository)
    context = ts.context("test-agent", request_id="request-1", claims={"tenant": "acme"},
                         budget=ts.budget(calls=4))
    value = rt.run(ts.create(Customer("c-1")), context)
    assert isinstance(value, Customer)
    assert seen == ["acme"]
    assert rt.run(ts.query(Customer), ctx()).items == (value,)


def test_policy_functions_filter_and_redact() -> None:
    backend = ts.MemoryBackend()
    seed = runtime(backend)
    seed.run(ts.create(Customer("visible", "visible@example.test", score=9)), ctx())
    seed.run(ts.create(Customer("hidden", "hidden@example.test", score=9)), ctx())

    def policy(request: ts.policy.Request) -> ts.policy.Decision:
        assert request.principal == "test-agent"
        return ts.policy.allow(where=ts.col("id") == "visible", redact=("email",))

    page = runtime(backend, policy=policy).run(ts.query(Customer), ctx())
    assert len(page.items) == 1
    assert page.items[0].id == "visible"
    assert page.items[0].email is None


def test_links_actions_aggregation_batch_and_introspection() -> None:
    rt = runtime(
        links={Referrals: lambda _context, source: ["b"] if source == "a" else []},
        actions={EchoCustomer: lambda _context, command: Customer(command.id)},
    )
    context = ctx()
    rt.run(ts.batch(ts.create(Customer("a", score=1)), ts.create(Customer("b", score=2))), context)
    linked = rt.run(ts.traverse(Referrals, "a"), context)
    assert [item.id for item in linked] == ["b"]
    groups = rt.run(ts.query(Customer).group_by("active").agg(total=ts.count()), context)
    assert groups.groups[0]["total"] == 2
    assert rt.run(EchoCustomer("a"), context).id == "a"
    assert rt.schema().objects[0]["api_name"] == "customer"
    assert rt.capabilities(context)["capabilities"]
    assert rt.explain(ts.query(Customer), context)["resource"] == "customer"


def test_invalid_declarations_fail_early() -> None:
    with pytest.raises(ValueError, match="effects"):
        @ts.action(returns=Customer)
        class InvalidAction:
            value: str

    with pytest.raises(ValueError, match="concurrency"):
        ts.action(returns=str, pure=True, concurrency="eventually")

    with pytest.raises(ValueError, match="approval"):
        ts.action(returns=str, pure=True, approval="sometimes")

    with pytest.raises(ValueError, match="risk"):
        ts.action(returns=str, pure=True, risk="catastrophic")

    with pytest.raises(ValueError, match="annotated"):
        @ts.object(key="missing")
        class InvalidObject:
            id: str


def test_atomic_batch_rejection_never_invokes_non_atomic_backend() -> None:
    calls = 0

    def create(_context: ts.Context, value: Customer) -> Customer:
        nonlocal calls
        calls += 1
        return value

    rt = runtime(ts.backend(create=create))
    with pytest.raises(ts.Error) as raised:
        rt.run(ts.batch(ts.create(Customer("a")), ts.create(Customer("b"))), ctx())
    assert raised.value.code == ts.ErrorCode.UNSUPPORTED_CAPABILITY
    assert calls == 0


def test_native_and_residual_aggregation_are_equivalent() -> None:
    rows = [Customer("a", score=1), Customer("b", score=2)]

    class NativeCustomers(ts.backend.Native[Customer]):
        def search(self, context: ts.Context, query):
            return rows

        def aggregate(self, context: ts.Context, query):
            return {"groups": [{"total": 2}]}

    plan = ts.query(Customer).group_by().agg(total=ts.count())
    pushed = runtime(NativeCustomers()).run(plan, ctx())
    residual = runtime(ts.backend(scan=lambda _context: rows)).run(plan, ctx())
    assert pushed == residual
