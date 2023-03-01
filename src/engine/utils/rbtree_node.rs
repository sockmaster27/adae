use std::cell::{Ref, RefCell, RefMut};

use intrusive_collections::{intrusive_adapter, KeyAdapter, RBTreeLink};

pub trait Keyed {
    type Key;
    fn key(&self) -> Self::Key;
}

pub struct TreeNode<T> {
    val: RefCell<T>,
    link: RBTreeLink,
}
impl<T> TreeNode<T> {
    pub fn new(val: T) -> Self {
        TreeNode {
            val: RefCell::new(val),
            link: RBTreeLink::new(),
        }
    }

    /// Immutably borrows the wrapped value.
    ///
    /// The borrow lasts until the returned Ref exits scope. Multiple immutable borrows can be taken out at the same time.
    ///
    /// # Panics
    /// Panics if the value is currently mutably borrowed.
    pub fn borrow(&self) -> Ref<'_, T> {
        self.val.borrow()
    }

    ///Mutably borrows the wrapped value.
    ///
    /// The borrow lasts until the returned RefMut or all RefMuts derived from it exit scope. The value cannot be borrowed while this borrow is active.
    ///
    /// # Panics
    /// Panics if the value is currently borrowed.
    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.val.borrow_mut()
    }
}

intrusive_adapter!(pub TreeNodeAdapter<T> = Box<TreeNode<T>>: TreeNode<T> { link: RBTreeLink });
impl<'a, T> KeyAdapter<'a> for TreeNodeAdapter<T>
where
    T: Keyed,
{
    type Key = T::Key;
    fn get_key(&self, node: &'a TreeNode<T>) -> T::Key {
        node.val.borrow().key()
    }
}
