use crate::{deleters, domain::DomainId, Deleter, HazPtrDomain, Reclaim};
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicPtr, Ordering},
};

pub trait HazPtrObject<'domain>
where
    Self: Sized + 'domain,
{
    fn domain(&self) -> &'domain HazPtrDomain;

    /// # Safety
    ///
    /// 1. Caller must guarantee that pointer is a valid reference.
    /// 2. Caller must guarantee that Self is no longer accessible to readers.
    /// 3. Caller must guarantee that the deleter is a valid deleter for Self.
    /// 4. Caller must guarantee that Self lives until the `HazPtrDomain` is dropped.
    ///
    /// It is okay for existing readers to still refer to Self.
    ///   
    unsafe fn retire(self: *mut Self, deleter: &'static dyn Deleter) {
        let ptr = self as *mut (dyn Reclaim + 'domain);
        unsafe {
            (&*self).domain().retire(ptr, deleter);
        }
    }

    fn into_ref<R>(self) -> R
    where
        R: HazPtrObjectRefExt<'domain, Self>,
    {
        HazPtrObjectRefExt::create(self)
    }
}

pub trait HazPtrObjectRef<'domain, O>
where
    O: HazPtrObject<'domain>,
{
    fn domain_id(&self) -> Option<&DomainId>;
    // TODO: make these messages more explicit:

    /// # Safety
    ///
    /// The pointer must not be changed to a state that would make it invalid to call `ptr.retire`
    unsafe fn ptr(&self) -> &AtomicPtr<O>;

    /// # Safety
    ///
    /// The pointer must not be changed to a state that would make it invalid to call `ptr.retire`
    unsafe fn ptr_mut(&mut self) -> &mut AtomicPtr<O>;
}

pub trait HazPtrObjectRefExt<'domain, O>: HazPtrObjectRef<'domain, O>
where
    O: HazPtrObject<'domain>,
{
    fn deleter(&self) -> &'static dyn Deleter;
    fn create(object: O) -> Self;

    // TODO: could we take `other: &Self` or would that cause
    //       unfixeable race conditions
    fn swap(&self, other: &mut Self, order: Ordering)
    where
        Self: Sized,
    {
        unsafe {
            assert_eq!(
                self.domain_id(),
                other.domain_id(),
                "tried to swap object with differing domains"
            );

            let other_ptr = other.ptr_mut().get_mut();
            let old = self.ptr().swap(*other_ptr, order);
            *other_ptr = old;
        }
    }

    fn replace(&self, other: O, order: Ordering) -> Self
    where
        Self: Sized,
    {
        let mut other = Self::create(other);
        self.swap(&mut other, order);
        other
    }

    // TODO: this function probably isn't actually safe
    fn retire(mut self)
    where
        Self: Sized,
    {
        // Safety:
        //
        // - we move owenership of the pointer so no one can create new references (?)
        // - the deleter is valid because of the trait guarantees
        // - the pointer is valid because of the trait guarantees
        unsafe {
            let deleter = self.deleter();
            let ptr = *self.ptr_mut().get_mut();
            ptr.retire(deleter);
        }
    }
}

pub struct AtomicBox<'domain, O>
where
    O: HazPtrObject<'domain>,
{
    ptr: AtomicPtr<O>,
    domain_id: DomainId,
    _phantom: PhantomData<&'domain ()>,
}

impl<'domain, O> HazPtrObjectRef<'domain, O> for AtomicBox<'domain, O>
where
    O: HazPtrObject<'domain>,
{
    fn domain_id(&self) -> Option<&DomainId> {
        Some(&self.domain_id)
    }

    unsafe fn ptr(&self) -> &AtomicPtr<O> {
        &self.ptr
    }

    unsafe fn ptr_mut(&mut self) -> &mut AtomicPtr<O> {
        &mut self.ptr
    }
}

impl<'domain, O> HazPtrObjectRefExt<'domain, O> for AtomicBox<'domain, O>
where
    O: HazPtrObject<'domain>,
{
    fn deleter(&self) -> &'static dyn Deleter {
        &deleters::drop_box
    }

    fn create(object: O) -> Self {
        Self {
            domain_id: unsafe { object.domain().id().duplicate() },
            ptr: AtomicPtr::new(Box::into_raw(Box::new(object))),
            _phantom: PhantomData,
        }
    }
}

impl<'domain, O> HazPtrObjectRef<'domain, O> for AtomicPtr<O>
where
    O: HazPtrObject<'domain>,
{
    fn domain_id(&self) -> Option<&DomainId> {
        None
    }

    unsafe fn ptr(&self) -> &AtomicPtr<O> {
        self
    }

    unsafe fn ptr_mut(&mut self) -> &mut AtomicPtr<O> {
        self
    }
}

pub struct HazPtrObjectWrapper<'domain, T> {
    inner: T,
    domain: &'domain HazPtrDomain,
}

impl<T> HazPtrObjectWrapper<'static, T> {
    pub fn with_global_domain(t: T) -> Self {
        HazPtrObjectWrapper::with_domain(HazPtrDomain::global(), t)
    }
}

impl<'domain, T> HazPtrObjectWrapper<'domain, T> {
    pub fn with_domain(domain: &'domain HazPtrDomain, t: T) -> Self {
        Self { inner: t, domain }
    }
}

impl<'domain, T: 'domain> HazPtrObject<'domain> for HazPtrObjectWrapper<'domain, T> {
    fn domain(&self) -> &'domain HazPtrDomain {
        self.domain
    }
}

impl<T> Deref for HazPtrObjectWrapper<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for HazPtrObjectWrapper<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
