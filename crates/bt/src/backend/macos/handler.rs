//! One Objective-C object receives every IOBluetooth callback (connect and
//! disconnect notifications, inquiry and pairing delegates). Each callback
//! only forwards a `Note`; the backend loop does the real work.

use super::Note;
use crate::model::PairPrompt;
use objc2::{
    AllocAnyThread, DefinedClass, define_class, msg_send, rc::Retained, runtime::AnyObject,
    runtime::NSObject, runtime::NSObjectProtocol,
};
use objc2_io_bluetooth::IOBluetoothDevice;
use std::sync::mpsc::Sender;

pub struct Ivars {
    notes: Sender<Note>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "TelmoBtHandler"]
    #[ivars = Ivars]
    pub struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    impl Handler {
        #[unsafe(method(connected:device:))]
        fn connected(&self, _note: Option<&AnyObject>, device: Option<&IOBluetoothDevice>) {
            if let Some(address) = device.and_then(super::address_of) {
                self.send(Note::Connected(address));
            }
        }

        #[unsafe(method(disconnected:device:))]
        fn disconnected(&self, _note: Option<&AnyObject>, _device: Option<&IOBluetoothDevice>) {
            self.send(Note::Disconnected);
        }

        #[unsafe(method(connectionComplete:status:))]
        fn connection_complete(&self, device: Option<&IOBluetoothDevice>, status: i32) {
            if let Some(address) = device.and_then(super::address_of) {
                self.send(Note::ConnectionComplete(address, status));
            }
        }

        #[unsafe(method(deviceInquiryDeviceFound:device:))]
        fn inquiry_found(&self, _sender: Option<&AnyObject>, _device: Option<&AnyObject>) {
            self.send(Note::Changed);
        }

        #[unsafe(method(deviceInquiryDeviceNameUpdated:device:devicesRemaining:))]
        fn inquiry_name(&self, _sender: Option<&AnyObject>, _device: Option<&AnyObject>, _left: u32) {
            self.send(Note::Changed);
        }

        #[unsafe(method(deviceInquiryComplete:error:aborted:))]
        fn inquiry_complete(&self, _sender: Option<&AnyObject>, _error: i32, _aborted: bool) {
            self.send(Note::InquiryComplete);
        }

        #[unsafe(method(devicePairingUserConfirmationRequest:numericValue:))]
        fn pairing_confirm(&self, _sender: Option<&AnyObject>, value: u32) {
            self.send(Note::Prompt(PairPrompt::Confirm(value)));
        }

        #[unsafe(method(devicePairingUserPasskeyNotification:passkey:))]
        fn pairing_passkey(&self, _sender: Option<&AnyObject>, passkey: u32) {
            self.send(Note::Prompt(PairPrompt::DisplayPasskey(passkey)));
        }

        #[unsafe(method(devicePairingPINCodeRequest:))]
        fn pairing_pin(&self, _sender: Option<&AnyObject>) {
            self.send(Note::Prompt(PairPrompt::EnterPin));
        }

        #[unsafe(method(devicePairingFinished:error:))]
        fn pairing_finished(&self, _sender: Option<&AnyObject>, error: i32) {
            self.send(Note::PairingFinished(error));
        }
    }
);

impl Handler {
    pub fn new(notes: Sender<Note>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { notes });
        unsafe { msg_send![super(this), init] }
    }

    fn send(&self, note: Note) {
        let _ = self.ivars().notes.send(note);
    }
}
