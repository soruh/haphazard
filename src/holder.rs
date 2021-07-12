use crate::HazPtrObjectRef;
use crate::{HazPtr, HazPtrDomain, HazPtrObject};
use std::sync::atomic::Ordering;

pub struct HazPtrHolder<'domain> {
    hazard: &'domain HazPtr,
    domain: &'domain HazPtrDomain,
}

impl HazPtrHolder<'static> {
    pub fn global() -> Self {
        HazPtrHolder::for_domain(HazPtrDomain::global())
    }
}

// This should really be the body of try_protect, and then protect should call try_protect,
// but that runs into a borrow checker limitation. See:
//
//  - https://github.com/rust-lang/rust/issues/51545
//  - https://github.com/rust-lang/rust/issues/54663
//  - https://github.com/rust-lang/rust/issues/58910
//  - https://github.com/rust-lang/rust/issues/84361
macro_rules! try_protect_actual {
    ($self:ident, $ptr:ident, $src:ident, $src_domain:ident) => {{
        if let Some(src_domain) = $src_domain {
            assert_eq!(
                $self.domain.id(),
                src_domain,
                "object guarded by different domain than holder used to access it"
            );
        }

        $self.hazard.protect($ptr as *mut u8);

        crate::asymmetric_light_barrier();

        let ptr2 = $src.load(Ordering::Acquire);
        if $ptr != ptr2 {
            $self.hazard.reset();
            Err(ptr2)
        } else {
            // All good -- protected
            Ok(std::ptr::NonNull::new($ptr).map(|nn| {
                // Safety: this is safe because:
                //
                //  1. Target of ptr1 will not be deallocated for the returned lifetime since
                //     our hazard pointer is active and pointing at ptr1.
                //  2. Pointer address is valid by the safety contract of load.
                let r = unsafe { nn.as_ref() };

                // The object reference did not save its domain.
                // If the domain was wrong we have already made a mistake,
                // let's hope this check catches the error (it may not).
                if $src_domain.is_none() {
                    debug_assert_eq!(
                        $self.domain as *const HazPtrDomain,
                        r.domain() as *const HazPtrDomain,
                        "object guarded by different domain than holder used to access it"
                    );
                }

                r
            }))
        }
    }};
}

impl<'domain> HazPtrHolder<'domain> {
    pub fn for_domain(domain: &'domain HazPtrDomain) -> Self {
        Self {
            hazard: domain.acquire(),
            domain,
        }
    }

    ///
    /// # Safety
    ///
    /// Caller must guarantee that the address in `AtomicPtr` is valid as a reference, or null.
    /// Caller must also guarantee that the value behind the `AtomicPtr` will only be deallocated
    /// through calls to [`HazPtrObject::retire`] on the same [`HazPtrDomain`] as this holder is
    /// associated with.
    pub unsafe fn protect<'l, 'o, O, R>(&'l mut self, src: &'_ R) -> Option<&'l O>
    where
        O: HazPtrObject<'o>,
        'o: 'l,
        R: HazPtrObjectRef<'o, O>,
    {
        // We are only reading the pointer in `src.ptr`
        let src_ptr = unsafe { src.ptr() };
        let src_domain = src.domain_id();

        let mut ptr = src_ptr.load(Ordering::Relaxed);
        loop {
            // Safety: same safety requirements as try_protect.
            // We are only reading the pointer in `src.ptr`
            match try_protect_actual!(self, ptr, src_ptr, src_domain) {
                Ok(r) => break r,
                Err(ptr2) => {
                    ptr = ptr2;
                }
            }
        }
    }

    ///
    /// # Safety
    ///
    /// Caller must guarantee that the address in `AtomicPtr` is valid as a reference, or null.
    /// Caller must also guarantee that the value behind the `AtomicPtr` will only be deallocated
    /// through calls to [`HazPtrObject::retire`] on the same [`HazPtrDomain`] as this holder is
    /// associated with.
    pub unsafe fn try_protect<'l, 'o, O, R>(
        &'l mut self,
        ptr: *mut O,
        src: &'_ R,
    ) -> Result<Option<&'l O>, *mut O>
    where
        'o: 'l,
        O: HazPtrObject<'o>,
        R: HazPtrObjectRef<'o, O>,
    {
        // We are only reading the pointer in `src.ptr`
        let src_ptr = unsafe { src.ptr() };
        let src_domain = src.domain_id();

        try_protect_actual!(self, ptr, src_ptr, src_domain)
    }

    pub fn reset(&mut self) {
        self.hazard.reset();
    }
}

impl Drop for HazPtrHolder<'_> {
    fn drop(&mut self) {
        self.hazard.reset();
        self.domain.release(self.hazard);
    }
}
