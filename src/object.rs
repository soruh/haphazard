use crate::{deleters, domain::DomainId, Deleter, HazPtrDomain, Reclaim};
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicPtr, Ordering},
};

pub trait HazPtrObject<'domain, F: 'static>
where
    Self: Sized + 'domain,
{
    fn domain(&self) -> &'domain HazPtrDomain<F>;

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
        R: HazPtrObjectRefExt<'domain, F, Self>,
    {
        HazPtrObjectRefExt::create(self)
    }
}

pub trait HazPtrObjectRef<'domain, F: 'static, O>
where
    O: HazPtrObject<'domain, F>,
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

pub trait HazPtrObjectRefExt<'domain, F: 'static, O>: HazPtrObjectRef<'domain, F, O>
where
    O: HazPtrObject<'domain, F>,
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

pub struct AtomicBox<'domain, F: 'static, O>
where
    O: HazPtrObject<'domain, F>,
{
    ptr: AtomicPtr<O>,
    domain_id: DomainId,
    _phantom: PhantomData<&'domain F>,
}

impl<'domain, F: 'static, O> HazPtrObjectRef<'domain, F, O> for AtomicBox<'domain, F, O>
where
    O: HazPtrObject<'domain, F>,
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

impl<'domain, F: 'static, O> HazPtrObjectRefExt<'domain, F, O> for AtomicBox<'domain, F, O>
where
    O: HazPtrObject<'domain, F>,
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

impl<'domain, F: 'static, O> HazPtrObjectRef<'domain, F, O> for AtomicPtr<O>
where
    O: HazPtrObject<'domain, F>,
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

pub struct HazPtrObjectWrapper<'domain, T, F> {
    inner: T,
    domain: &'domain HazPtrDomain<F>,
}

impl<T> HazPtrObjectWrapper<'static, T, crate::Global> {
    pub fn with_global_domain(t: T) -> Self {
        HazPtrObjectWrapper::with_domain(HazPtrDomain::global(), t)
    }
}

impl<'domain, T, F> HazPtrObjectWrapper<'domain, T, F> {
    pub fn with_domain(domain: &'domain HazPtrDomain<F>, t: T) -> Self {
        Self { inner: t, domain }
    }
}

impl<'domain, T: 'domain, F: 'static> HazPtrObject<'domain, F>
    for HazPtrObjectWrapper<'domain, T, F>
{
    fn domain(&self) -> &'domain HazPtrDomain<F> {
        self.domain
    }
}

impl<T, F> Deref for HazPtrObjectWrapper<'_, T, F> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T, F> DerefMut for HazPtrObjectWrapper<'_, T, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
