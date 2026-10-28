use tesela::backend::{Native, native};
use tesela::prelude::*;
use tesela::query;
use tesela::raw::Query as RawQuery;

#[derive(Debug, Clone, PartialEq, Object)]
#[tesela(key = id)]
struct Customer {
    id: String,
    region: Option<String>,
    spend: i64,
}

fn customers(count: usize) -> Vec<Customer> {
    (0..count)
        .map(|index| Customer {
            id: format!("c-{index:03}"),
            region: (index % 7 != 0).then(|| ["north", "south", "east"][index % 3].to_owned()),
            spend: i64::try_from((index * 37) % 101).expect("small"),
        })
        .collect()
}

fn context() -> Result<Context, Error> {
    Context::builder("test").request_id("repository").build()
}

struct Slice(Vec<Customer>);

impl Native<Customer> for Slice {
    fn search(&self, _: &Context, query: &RawQuery) -> Result<Vec<Customer>, Error> {
        let offset = usize::try_from(query.offset.unwrap_or(0)).unwrap_or(0);
        let limit = usize::try_from(query.limit.unwrap_or(i32::MAX)).unwrap_or(0);
        Ok(self.0.iter().skip(offset).take(limit).cloned().collect())
    }
}

#[test]
fn a_native_backend_reports_a_cursor_only_while_rows_remain() -> Result<(), Error> {
    let context = context()?;
    for (count, expected) in [
        (10, [Some("4"), Some("8"), None]),
        (8, [Some("4"), None, None]),
    ] {
        let runtime = Runtime::builder()
            .object::<Customer>(native(Slice(customers(count))))
            .policy(tesela::policy::allow_all)
            .build()?;
        let mut cursors = Vec::new();
        for page in 0..3 {
            let result = runtime.run(query::<Customer>().limit(4).offset(page * 4), &context)?;
            cursors.push(result.next_cursor);
            assert!(result.items.len() <= 4);
        }
        let cursors: Vec<_> = cursors.iter().map(Option::as_deref).collect();
        assert_eq!(cursors, expected);
    }
    Ok(())
}
