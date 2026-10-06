#![no_std]
#![forbid(unsafe_code)]

pub fn known_ticket() -> u16 {
    Some(17).unwrap()
}

pub fn optional_ticket(available: bool) -> u16 {
    let ticket = if available { Some(17) } else { None };
    ticket.unwrap()
}

pub fn expected_ticket(available: bool) -> u16 {
    let ticket = if available { Some(17) } else { None };
    ticket.expect("ticket missing")
}

pub fn guarded_ticket(available: bool) -> u16 {
    let ticket = if available { Some(17) } else { None };
    if ticket.is_some() { ticket.unwrap() } else { 0 }
}

mod option {
    pub fn unwrap_failed() -> u16 {
        23
    }
    pub fn expect_failed(_label: &str) -> u16 {
        41
    }
}

mod idle_option {
    pub fn unwrap_failed() -> ! {
        loop {}
    }
}

pub fn application_nonreturning_helper() {
    idle_option::unwrap_failed();
}

pub fn application_helper_names() {
    assert!(option::unwrap_failed() == 23);
}

pub fn application_message_helper() {
    assert!(option::expect_failed("user helper") == 41);
}

pub fn bad_application_helper_names() {
    assert!(option::unwrap_failed() == 17);
}

pub fn primitive_advance(total: u32) {
    if total < 1000 {
        let mut running = total;
        core::ops::AddAssign::add_assign(&mut running, 7);
        assert!(running == total + 7);
    }
}

pub fn unresolved_ticket(provider: fn() -> u16) -> u16 {
    provider()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_guards_and_counter_updates_match_native_rust() {
        assert_eq!(known_ticket(), 17);
        assert_eq!(optional_ticket(true), 17);
        assert_eq!(expected_ticket(true), 17);
        assert_eq!(guarded_ticket(true), 17);
        assert_eq!(guarded_ticket(false), 0);
        application_helper_names();
        application_message_helper();
        for total in 0..=1000 {
            primitive_advance(total);
        }
    }
}
