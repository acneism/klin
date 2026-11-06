/* This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
* file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use crate::support::interrupt::Interruptee;
use parking_lot::Mutex;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::support::interrupt::SqlInterruptHandle;

static IN_SHUTDOWN: AtomicBool = AtomicBool::new(false);

lazy_static::lazy_static! {
   static ref REGISTERED_INTERRUPTS: Mutex<Vec<Weak<dyn AsRef<SqlInterruptHandle> + Send + Sync>>> = Mutex::new(Vec::new());
}

pub fn shutdown() {
    IN_SHUTDOWN.store(true, Ordering::Relaxed);
    for weak in REGISTERED_INTERRUPTS.lock().iter() {
        if let Some(interrupt) = weak.upgrade() {
            interrupt.as_ref().as_ref().interrupt()
        }
    }
}

pub fn in_shutdown() -> bool {
    IN_SHUTDOWN.load(Ordering::Relaxed)
}

pub fn register_interrupt(interrupt: Weak<dyn AsRef<SqlInterruptHandle> + Send + Sync>) {
    let mut interrupts = REGISTERED_INTERRUPTS.lock();
    for weak in interrupts.iter_mut() {
        if weak.strong_count() == 0 {
            *weak = interrupt;
            return;
        }
    }

    interrupts.push(interrupt);
}

pub struct ShutdownInterruptee;
impl Interruptee for ShutdownInterruptee {
    #[inline]
    fn was_interrupted(&self) -> bool {
        in_shutdown()
    }
}
