# pyright: strict
from typing import Mapping, assert_type
import tesela as ts


@ts.object(key="id")
class Customer:
    id: str
    email: str | None = None


@ts.object(key="id")
class Order:
    id: str
    customer_id: str


orders = ts.link("customer_orders", Customer, Order)


customer = Customer("c-1", email="a@example.test")
plan = ts.query(Customer).where(ts.col("email") == "a@example.test")
assert_type(plan, ts.Query[Customer])

runtime = ts.Runtime({Customer: ts.MemoryBackend()}, policy=ts.policy.allow_all)
context = ts.context("type-check", request_id="type-check-1")
assert_type(runtime.run(plan, context), ts.Page[Customer])
assert_type(runtime.run(ts.get(Customer, "c-1"), context), Customer | None)
assert_type(runtime.run(ts.create(customer), context), Customer)
assert_type(runtime.execute(plan, context), ts.Execution[ts.Page[Customer]])
assert_type(ts.traverse(orders, "c-1"), ts.Traverse[Order])


class NativeOrders(ts.backend.Native[Order]):
    def search(self, context: ts.Context, query: Mapping[str, object]) -> list[Order]:
        return []
