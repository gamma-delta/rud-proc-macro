use std::sync::{Arc, Mutex};

use rlua::{Lua, MetaMethod, UserData, UserDataMethods, Value};
use rud_proc_macro::UserData;

#[test]
fn attributes() {
    #[derive(UserData)]
    struct Tester {
        #[userdata]
        field: u32,
        #[userdata(read)]
        read_only: String,
        #[userdata(rename = "hello", read)]
        hellont: isize,
    }
    let _s = Tester {
        field: 10,
        read_only: String::from("read only?!"),
        hellont: -44,
    };
}

#[test]
fn nested() {
    #[derive(UserData)]
    struct Foo {
        #[userdata]
        bar: Arc<Mutex<Bar>>,
    }
    #[derive(UserData, Default)]
    struct Bar {
        #[userdata]
        value: i32,
    }

    let lua = Lua::new();
    lua.context(|ctx| -> rlua::Result<()> {
        let globals = ctx.globals();
        let foo = Foo {
            bar: Arc::new(Mutex::new(Bar { value: 0 })),
        };
        globals.set("foo", foo).unwrap();

        let ending_val = ctx
            .load(
                r#"
                assert(foo.bar.value == 0)
                foo.bar.value = 10
                assert(foo.bar.value == 10)

                -- this clones the Arc
                local bar_alias = foo.bar
                for i = 1,1000 do
                    bar_alias.value = i
                    assert(foo.bar.value == i)
                end

                return foo.bar.value
            "#,
            )
            .eval::<i32>()
            .unwrap();
        assert_eq!(ending_val, 1000);

        Ok(())
    })
    .unwrap();
}

#[test]
fn empty() {
    #[derive(UserData)]
    struct Foo {
        field1: i32,
        field2: i32,
    }
}

#[test]
fn generics() {
    #[derive(UserData)]
    struct Foo<'a, T>
    where
        T: std::fmt::Display,
    {
        reference: &'a i32,
        displayer: T,
        #[userdata]
        accessible_field: String,
    }
}
