"""Public Python facade microbenchmarks. Run with `uv run python benchmarks/baseline.py`."""
from time import perf_counter
import tesela as ts


@ts.object(key="id")
class Customer:
    id: str
    score: int


def measure(label, work, count=10_000):
    samples = []
    for _ in range(3):
        start = perf_counter()
        for _ in range(count):
            work()
        samples.append((perf_counter() - start) * 1_000_000 / count)
    samples.sort()
    print(f"{label}: {samples[1]:.3f} us/op median ({count} ops/run)")


def main():
    context = ts.context("benchmark", request_id="benchmark")
    runtime = ts.Runtime({Customer: ts.MemoryBackend()}, policy=ts.policy.allow_all)
    for number in range(100):
        runtime.run(ts.create(Customer(str(number), number)), context)
    query = ts.query(Customer).where(ts.col("score") >= 50).limit(25)
    all_rows = ts.query(Customer)
    aggregate = ts.query(Customer).group_by().agg(total=ts.count())
    measure("plan.query", lambda: ts.query(Customer).where(ts.col("score") >= 50).limit(25))
    measure("query.bounded", lambda: runtime.run(query, context), 2_000)
    measure("query.100", lambda: runtime.run(all_rows, context), 2_000)
    measure("get", lambda: runtime.run(ts.get(Customer, "50"), context), 5_000)
    measure("upsert", lambda: runtime.run(ts.upsert(Customer("0", 1)), context), 2_000)
    measure("aggregate", lambda: runtime.run(aggregate, context), 2_000)


if __name__ == "__main__":
    main()
