#![warn(unsafe_op_in_unsafe_fn)]

use haphazard::deleters::drop_box;
use haphazard::*;

use std::sync::atomic::AtomicPtr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct CountDrops(Arc<AtomicUsize>);
impl Drop for CountDrops {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn feels_good() {
    let drops_42 = Arc::new(AtomicUsize::new(0));

    let x = HazPtrObjectWrapper::with_global_domain((42, CountDrops(Arc::clone(&drops_42))))
        .into_ref::<AtomicBox<_, _>>();

    // As a reader:
    let mut h = HazPtrHolder::global();

    // Safety:
    //
    //  1. AtomicPtr points to a Box, so is always valid.
    //  2. Writers to AtomicPtr use HazPtrObject::retire.
    let my_x = unsafe { h.protect(&x) }.expect("not null");
    // valid:
    assert_eq!(my_x.0, 42);
    h.reset();
    // invalid:
    // let _: i32 = my_x.0;

    let my_x = unsafe { h.protect(&x) }.expect("not null");
    // valid:
    assert_eq!(my_x.0, 42);
    drop(h);
    // invalid:
    // let _: i32 = my_x.0;

    let mut h = HazPtrHolder::global();
    let my_x = unsafe { h.protect(&x) }.expect("not null");

    let mut h_tmp = HazPtrHolder::global();
    let _ = unsafe { h_tmp.protect(&x) }.expect("not null");
    drop(h_tmp);

    // As a writer:
    let drops_9001 = Arc::new(AtomicUsize::new(0));
    let old: AtomicBox<_, _> = x.replace(
        HazPtrObjectWrapper::with_global_domain((9001, CountDrops(Arc::clone(&drops_9001)))),
        std::sync::atomic::Ordering::SeqCst,
    );

    let mut h2 = HazPtrHolder::global();
    let my_x2 = unsafe { h2.protect(&x) }.expect("not null");

    assert_eq!(my_x.0, 42);
    assert_eq!(my_x2.0, 9001);

    old.retire();

    assert_eq!(drops_42.load(Ordering::SeqCst), 0);
    assert_eq!(my_x.0, 42);

    let n = HazPtrDomain::global().eager_reclaim();
    assert_eq!(n, 0);

    assert_eq!(drops_42.load(Ordering::SeqCst), 0);
    assert_eq!(my_x.0, 42);

    drop(h);
    assert_eq!(drops_42.load(Ordering::SeqCst), 0);
    // _not_ drop(h2);

    let n = HazPtrDomain::global().eager_reclaim();
    assert_eq!(n, 1);

    assert_eq!(drops_42.load(Ordering::SeqCst), 1);
    assert_eq!(drops_9001.load(Ordering::SeqCst), 0);

    drop(h2);
    let n = HazPtrDomain::global().eager_reclaim();
    assert_eq!(n, 0);
    assert_eq!(drops_9001.load(Ordering::SeqCst), 0);
}

#[test]
#[should_panic]
fn feels_bad() {
    let dw = HazPtrDomain::next(&());
    let dr = HazPtrDomain::next(&());

    let drops_42 = Arc::new(AtomicUsize::new(0));

    let x = HazPtrObjectWrapper::with_domain(&dw, (42, CountDrops(Arc::clone(&drops_42))))
        .into_ref::<AtomicBox<_, _>>();

    // Reader uses a different domain thant the writer!
    let mut h = HazPtrHolder::for_domain(&dr);

    // This should always catch the error (at least in debug mode).
    let _ = unsafe { h.protect(&x) }.expect("not null");
}

#[test]
fn atomic_ptr_as_object_ref() {
    let drops = Arc::new(AtomicUsize::new(0));

    let domain = HazPtrDomain::next(&());

    let mut hx = HazPtrHolder::for_domain(&domain);
    let mut hy = HazPtrHolder::for_domain(&domain);

    let x = HazPtrObjectWrapper::with_domain(&domain, (0, CountDrops(drops.clone())));
    let x = Box::into_raw(Box::new(x));

    // This pointer changes its backing allocation from `Box` to `Arc`
    // so it can not have a single reference type.
    let p = AtomicPtr::new(x);

    let rx = unsafe { hx.protect(&p) }.expect("not null");
    assert_eq!(rx.0, 0);

    let y = HazPtrObjectWrapper::with_domain(&domain, (1, CountDrops(drops.clone())));
    let y = Arc::into_raw(Arc::new(y)) as *mut _;

    let old = p.swap(y, Ordering::SeqCst);
    assert_eq!(old, x);

    let ry = unsafe { hy.protect(&p) }.expect("not null");
    assert_eq!(rx.0, 0);
    assert_eq!(ry.0, 1);

    unsafe fn _drop_arc(ptr: *mut dyn Reclaim) {
        let _ = unsafe { Arc::from_raw(ptr as *const _) };
    }

    #[allow(non_upper_case_globals)]
    pub const drop_arc: unsafe fn(*mut dyn Reclaim) = _drop_arc;

    unsafe {
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        x.retire(&drop_box);

        assert_eq!(drops.load(Ordering::SeqCst), 0);

        drop(hx);

        assert_eq!(drops.load(Ordering::SeqCst), 0);

        assert_eq!(domain.eager_reclaim(), 1);

        assert_eq!(drops.load(Ordering::SeqCst), 1);

        y.retire(&drop_arc);

        assert_eq!(drops.load(Ordering::SeqCst), 1);

        drop(hy);

        assert_eq!(drops.load(Ordering::SeqCst), 1);

        assert_eq!(domain.eager_reclaim(), 1);

        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
}
