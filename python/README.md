# Python bindings

The Python package is a curated facade over `tesela._native`. Native storage, compilation, authority checks, policy enforcement, and execution remain in Rust. Python owns declaration ergonomics, typed model materialization, query expressions, and callback adaptation.

The layout deliberately separates responsibilities:

- `crates/tesela-py` is the mechanical PyO3 boundary;
- `python/tesela/__init__.py` is the small public Python vocabulary;
- `python/tesela/model.py` and `runtime.py` own declarations and execution ergonomics;
- checked-in `.pyi` files and `py.typed` define the verified typing contract;
- `python/tests` verifies import shape, typed values, callbacks, and packaging.

The package is a native language binding, not a second client-layer architecture, and it has no JSON-string ABI. `Spec.to_json()` exists only for explicit interchange.

Build and verify with `maturin develop`, `pytest python/tests -q`, `pyright`, and `PYTHONPATH=python pyright --verifytypes tesela --ignoreexternal` from the repository root.
