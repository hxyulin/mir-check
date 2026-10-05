#![forbid(unsafe_code)]

use mir_contracts::{ensures, no_panic, requires};

#[no_panic]
#[requires(unresolved_name > 0)]
#[ensures(result == unresolved_name)]
const fn identity(value: u8) -> u8 {
    value
}

struct Value(u8);

impl Value {
    #[requires(false)]
    fn read(&self) -> u8 {
        self.0
    }
}

#[test]
fn predicates_add_no_runtime_checks_or_name_resolution() {
    const ZERO: u8 = identity(0);
    assert_eq!(ZERO, 0);
    assert_eq!(Value(9).read(), 9);
}
