"""Public Python-path benchmark using direct PyO3 values."""

from dataclasses import dataclass
from time import perf_counter

from tesela import MemoryBackend, Runtime, Spec


def measure(label, work, count=10_000):
    start = perf_counter()
    for _ in range(count):
        work()
    print(f"{label}: {(perf_counter() - start) * 1_000_000 / count:.3f} us/op ({count} ops)")


def main():
    spec = Spec()

    @spec.object_type(primary_key="id")
    @dataclass
    class Customer:
        id: str
        score: int

    runtime = Runtime(spec, objects={Customer: MemoryBackend()}, policy="allow_all")
    for number in range(100):
        runtime.mutate(Customer, {"create": {"values": {"id": str(number), "score": number}}})
    document = spec.to_json()
    update = {"upsert": {"values": {"id": "0", "score": 1}}}
    measure("spec.parse", lambda: Spec.from_json(document))
    measure("spec.serialize", spec.to_json)
    measure("search.100", lambda: runtime.search(Customer))
    measure("get", lambda: runtime.get(Customer, "0"))
    measure("mutate.upsert", lambda: runtime.mutate(Customer, update))


if __name__ == "__main__":
    main()
