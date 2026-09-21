# Python facade

The Python API is a typed facade over the Rust engine. Its public model has three parts:

- immutable definitions: `@tesela.object`, links, and `@tesela.action` commands;
- immutable expressions and plans: `col`, `query`, `get`, mutations, traversal, aggregation, and batch;
- `Runtime`, the only execution boundary.

Every call to `run` or `execute` requires an explicit immutable `Context`. `run` returns the useful value; `execute` additionally returns the receipt and optional revision. Runtime construction discovers semantic definitions from typed binding maps and does not use a registry or public `Spec`.

Application typing is defined by the inline Python source. The only stub is [`_native.pyi`](tesela/_native.pyi), because the extension is compiled. [`consumer.py`](typecheck/consumer.py) is a strict consumer fixture checked by Pyright.

Run:

```text
uv run --group dev pytest python/tests -q
uv run --group dev pyright
uv build
```
