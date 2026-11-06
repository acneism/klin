/* This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
* file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::support::interrupt::Interrupted;

pub trait Interruptee {
    fn was_interrupted(&self) -> bool;

    fn err_if_interrupted(&self) -> Result<(), Interrupted> {
        if self.was_interrupted() {
            return Err(Interrupted);
        }
        Ok(())
    }
}

pub struct NeverInterrupts;

impl Interruptee for NeverInterrupts {
    #[inline]
    fn was_interrupted(&self) -> bool {
        false
    }
}
