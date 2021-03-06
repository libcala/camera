use smelling_salts::{Device, Watcher};

use std::{
    convert::TryInto,
    future::Future,
    mem::{size_of, MaybeUninit},
    os::{
        raw::{c_void, c_int, c_ulong, c_long, c_char},
        unix::{fs::OpenOptionsExt, io::IntoRawFd},
    },
    ptr::null_mut,
    pin::Pin,
    task::{Context, Poll},
    fs::{self, OpenOptions},
    collections::HashSet,
    io::ErrorKind,
};
use pix::rgb::SRgba8;
use pix::Raster;

#[repr(C)]
struct InotifyEv {
    // struct inotify_event, from C.
    wd: c_int, /* Watch descriptor */
    mask: u32, /* Mask describing event */
    cookie: u32, /* Unique cookie associating related
               events (for rename(2)) */
    len: u32,            /* Size of name field */
    name: [c_char; 256], /* Optional null-terminated name */
}

#[repr(C)]
struct TimeVal {
    // struct timeval, from C.
    tv_sec: c_long,
    tv_usec: c_long,
}

/// Type of the buffer
#[repr(C)]
#[allow(unused)]
enum V4l2BufType {
    /// Buffer of a single-planar video capture stream, see Video Capture Interface.
    VideoCapture =	1,
    /// Buffer of a multi-planar video capture stream, see Video Capture Interface.
    VideoCaptureMPlane = 9,
    /// Buffer of a single-planar video output stream, see Video Output Interface.
    VideoOutput = 2,
    /// Buffer of a multi-planar video output stream, see Video Output Interface.
    VideoOutputMPlane =	10,
    /// Buffer for video overlay, see Video Overlay Interface.
    VideoOverlay =	3, 	
    /// Buffer of a raw VBI capture stream, see Raw VBI Data Interface.
    VbiCapture = 	4,
    /// Buffer of a raw VBI output stream, see Raw VBI Data Interface.
    VbiOutput =	5,
    /// Buffer of a sliced VBI capture stream, see Sliced VBI Data Interface.
    SlicedVbiCapture =	6,
    /// Buffer of a sliced VBI output stream, see Sliced VBI Data Interface.
    SlicedVbiOutput =	7,
    /// Buffer for video output overlay (OSD), see Video Output Overlay Interface.
    VideoOutputOverlay =	8,
    /// Buffer for Software Defined Radio (SDR) capture stream, see Software Defined Radio Interface (SDR).
    SdrCapture =	11,
    /// Buffer for Software Defined Radio (SDR) output stream, see Software Defined Radio Interface (SDR).
    SdrOutput =	12,
}

#[repr(C)]
struct V4l2Capability {
    driver: [u8; 16],    /* i.e. "bttv" */
    card: [u8; 32],      /* i.e. "Hauppauge WinTV" */
    bus_info: [u8; 32],  /* "PCI:" + pci_name(pci_dev) */
    version: u32,        /* should use KERNEL_VERSION() */
    capabilities: u32,   /* Device capabilities */
    reserved: [u32; 4],
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(unused)]
enum V4l2Field {
    /// Driver can choose from none, top, bottom, interlaced depending on
    /// whatever it thinks is approximate ...
    Any = 0,
    /// This device has no fields
    None = 1,
    /// Top field only
    Top = 2,
    /// Bottom field only
    Bottom = 3,
    /// Both fields interlaced
    Interlaced = 4,
    /// Both fields sequential into one buffer, top-bottom order
    SeqTopBottom = 5,
    /// Same as above + bottom-top order
    SeqBottomTop = 6,
    /// Both fields alternating into separate buffers
    Alternate = 7,
}

#[repr(C)]
#[derive(Copy, Clone)]
#[allow(unused)]
enum V4l2Colorspace {
    /// Let the driver choose
    Unset = 0,
    /// ITU-R 601 -- broadcast NTSC/PAL
    Smpte170M = 1,
    /// 1125-Line (US) HDTV
    Smpte240M = 2,
    /// HD and modern captures.
    Rec709 = 3,
    /// broken BT878 extents (601, luma range 16-253 instead of 16-235)
    Bt878 = 4,
    /// These should be useful.  Assume 601 extents.
    System470M  = 5,
    System470BG = 6,
    /// I know there will be cameras that send this.  So, this is
    /// unspecified chromaticities and full 0-255 on each of the
    /// Y'CbCr components
    Jpeg = 7,
    /// For RGB colourspaces, this is probably a good start.
    Srgb = 8,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct V4l2PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: V4l2Field,
    bytesperline: u32, /* for padding, zero if unused */
    sizeimage: u32,
    colorspace: V4l2Colorspace,
    private: u32,       /* private data, depends on pixelformat */
}

#[repr(C)]
#[derive(Copy, Clone)]
struct V4l2Rect {
     left: i32,
     top: i32,
     width: i32,
     height: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct V4l2Clip {
    c: V4l2Rect,
    next: *mut V4l2Clip,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct V4l2Window {
     w: V4l2Rect,
     field: V4l2Field,
     chromakey: u32,
     clips: *mut V4l2Clip,
     clipcount: u32,
     bitmap: *mut c_void,
}

#[repr(C)]
struct V4l2Timecode {
    type_: u32,
    flags: u32,
    frames: u8,
    seconds: u8,
    minutes: u8,
    hours: u8,
    userbits: [u8; 4],
}

#[repr(C)]
union V4l2BufferUnion {
    offset: u32,
    userptr: c_ulong
}

#[repr(C)]
struct V4l2Buffer {
    index: u32,
    type_: V4l2BufType,
    bytesused: u32,
    flags: u32,
    field: V4l2Field,
    timestamp: TimeVal,
    timecode: V4l2Timecode,
    sequence: u32,

    /* memory location */
    memory: V4l2Memory,
    m: V4l2BufferUnion,
    length: u32,
    input: u32,
    reserved: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct V4l2VbiFormat {
    sampling_rate: u32,     /* in 1 Hz */
    offset: u32,
    samples_per_line: u32,
    sample_format: u32,     /* V4L2_PIX_FMT_* */
    start: [i32; 2],
    count: [u32; 2],
    flags: u32,             /* V4L2_VBI_* */
    reserved: [u32; 2],     /* must be zero */
}

#[repr(C)]
union V4l2FormatUnion {
    pix: V4l2PixFormat,     // V4l2BufType::VideoCapture
    win: V4l2Window,        // V4l2BufType::VideoOverlay
    vbi: V4l2VbiFormat,     // V4l2BufType::VbiCapture
    raw_data: [u8; 200],    // user-defined
}

/// Stream data format
#[repr(C)]
struct V4l2Format {
    type_: V4l2BufType,
    fmt: V4l2FormatUnion,
}

#[repr(C)]
#[allow(unused)]
enum V4l2Memory {
    Mmap = 1,
    UserPtr = 2,
    MemoryOverlay = 3,
}

#[repr(C)]
struct V4l2RequestBuffers {
    count: u32,
    type_: V4l2BufType,
    memory: V4l2Memory,
    reserved: [u32; 2],
}

/// IOCTL
const fn iow_v(size: usize, num: u8) -> c_ulong {
    (0x80 << 24) | ((size as c_ulong & 0x1fff) << 16) | ((b'V' as c_ulong) << 8) | num as c_ulong
}
const fn ior_v(size: usize, num: u8) -> c_ulong {
    (0x40 << 24) | ((size as c_ulong & 0x1fff) << 16) | ((b'V' as c_ulong) << 8) | num as c_ulong
}
const fn iowr_v(size: usize, num: u8) -> c_ulong {
    (0xc0 << 24) | ((size as c_ulong & 0x1fff) << 16) | ((b'V' as c_ulong) << 8) | num as c_ulong
}
const VIDIOC_STREAMON: c_ulong = iow_v(size_of::<c_int>(), 18);
const VIDIOC_STREAMOFF: c_ulong = iow_v(size_of::<c_int>(), 19);
const VIDIOC_QUERYCAP: c_ulong = ior_v(size_of::<V4l2Capability>(), 0);
const VIDIOC_S_FMT: c_ulong = iowr_v(size_of::<V4l2Format>(), 5);
const VIDIOC_REQBUFS: c_ulong = iowr_v(size_of::<V4l2RequestBuffers>(), 8);
const VIDIOC_QUERYBUF: c_ulong = iowr_v(size_of::<V4l2Buffer>(), 9);
const VIDIOC_QBUF: c_ulong = iowr_v(size_of::<V4l2Buffer>(), 15);
const VIDIOC_DQBUF: c_ulong = iowr_v(size_of::<V4l2Buffer>(), 17);

const fn v4l2_fourcc(a: &[u8; 4]) -> u32 {
    ((a[0] as u32)<<0)|((a[1] as u32)<<8)|((a[2] as u32)<<16)|((a[3] as u32)<<24)
}

const V4L2_PIX_FMT_MJPEG: u32 = v4l2_fourcc(b"MJPG");

const PROT_READ: c_int = 0x04;
const PROT_WRITE: c_int = 0x02;

const MAP_SHARED: c_int = 0x0010;

fn xioctl(fd: c_int, request: c_ulong, arg: *mut c_void) -> c_int {
    // Keep going until syscall is not interrupted.
    loop {
        match unsafe { ioctl(fd, request, arg) } {
            -1 if errno() == 4 /*EINTR*/ => {}
            r => break r,
        }
    }
}

#[inline(always)]
fn errno() -> c_int {
    unsafe { *__errno_location() }
}

extern "C" {
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn mmap(addr: *mut c_void, length: usize, prot: c_int, flags: c_int,
        fd: c_int, offset: isize) -> *mut c_void;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
    fn close(fd: c_int) -> c_int;
    fn __errno_location() -> *mut c_int;
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
}

/// 
pub enum Event {
    Connect(Box<Camera>),
}

/// All cameras / webcams that are connected to the operating system.
pub struct Rig {
    device: Device,
    connected: HashSet<String>,
}

impl Rig {
    pub fn new() -> Self {
        // Create an inotify on the directory where video inputs are.
        let inotify = unsafe {
            inotify_init1(0o0004000 /*IN_NONBLOCK*/)
        };
        if inotify == -1 {
            panic!("Couldn't create inotify (1)!");
        }
        if unsafe {
            inotify_add_watch(
                inotify,
                b"/dev/\0".as_ptr() as *const _,
                0x0000_0200 | 0x0000_0100,
            )
        } == -1
        {
            panic!("Couldn't create inotify (2)!");
        }

        // Create watcher, and register with fd as a "device".
        let watcher = Watcher::new().input();
        let device = Device::new(inotify, watcher);

        // Start off with an empty hash set of connected devices.
        let connected = HashSet::new();

        // Return
        Rig {
            device,
            connected,
        }
    }
}

impl Future for Rig {
    type Output = Camera;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Camera> {
        // Read an event.
        let mut ev = MaybeUninit::<InotifyEv>::uninit();
        let ev = unsafe {
            if read(
                self.device.fd(),
                ev.as_mut_ptr().cast(),
                std::mem::size_of::<InotifyEv>(),
            ) <= 0
            {
                let mut all_open = true;
                // Search directory for new video inputs.
                'fds: for file in fs::read_dir("/dev/").unwrap() {
                    let file = file.unwrap().file_name().into_string().unwrap();
                    if file.starts_with("video") {
                        // Found a camera
                        if self.connected.contains(&file) {
                            // Already connected.
                            continue 'fds;
                        }
                        // New gamepad
                        let mut filename = "/dev/".to_string();
                        filename.push_str(&file);
                        let fd = match OpenOptions::new()
                            .read(true)
                            .append(true)
                            .open(filename)
                        {
                            Ok(f) => f,
                            Err(e) => {
                                if e.kind() == ErrorKind::PermissionDenied {
                                    all_open = false;
                                }
                                continue 'fds;
                            }
                        };
                        self.connected.insert(file);
                        if let Some(camera) = Camera::new(fd.into_raw_fd(), Raster::with_clear(640, 480)) {
                            return Poll::Ready(
                                camera
                            );
                        }
                    }
                }
                // Register waker for this device
                self.device.register_waker(cx.waker());
                // If no new controllers found, return pending.
                return Poll::Pending;
            }
            ev.assume_init()
        };

        // Remove flag is set, remove from HashSet.
        if (ev.mask & 0x0000_0200) != 0 {
            let mut file = "".to_string();
            let name = unsafe { std::ffi::CStr::from_ptr(ev.name.as_ptr()) };
            file.push_str(&name.to_string_lossy());
            if file.ends_with("-event-joystick") {
                // Remove it if it exists, sometimes gamepads get "removed"
                // twice because adds are condensed in innotify (not 100% sure).
                let _ = self.connected.remove(&file);
            }
        }
        // Check for more events, Search for new controllers again, and return
        // Pending if neither have anything to process.
        self.poll(cx)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let fd = self.device.fd();
        self.device.old();
        assert_ne!(unsafe { close(fd) }, -1);
    }
}

/// A camera / webcam in the `Rig`.
pub struct Camera {
    // Camera device to watch for events.
    device: Device,

	// Linux specific
	buffer: *mut c_void,
	buf: V4l2Buffer,

	// 
	data: *mut c_void, // JPEG file data
	size: u32, // Size of JPEG file
	
	// SRGB camera frame data.
	raster: Raster<SRgba8>,
}

impl Camera {
    pub fn new(fd: c_int, raster: Raster<SRgba8>) -> Option<Camera> {
	    // Open the device
        let filename = "/dev/video0";
        let fd = match OpenOptions::new()
            .read(true)
            .append(true)
            .mode(0)
            .custom_flags(0x0004 /*O_NONBLOCK*/)
            .open(filename)
        {
            Ok(f) => f.into_raw_fd(),
            Err(_e) => return None,
        };
        if fd == -1 {
            return None;
        }
        // FIXME: Do I need to set asynchronous on FD?

	    

	    // Is it available?
	    let mut caps: MaybeUninit<V4l2Capability> = MaybeUninit::uninit();
	    if xioctl(fd, VIDIOC_QUERYCAP, caps.as_mut_ptr().cast()) == -1 {
		    panic!("Failed Querying Capabilites\n");
	    }

	    // Set image format.
	    let mut fmt = V4l2Format {
	        type_: V4l2BufType::VideoCapture,
	        fmt: V4l2FormatUnion {
	            pix: V4l2PixFormat {
            	    width: 0, // w,
	                height: 0, // h,
	                pixelformat: V4L2_PIX_FMT_MJPEG,
	                field: V4l2Field::None,
                    bytesperline: 0,
                    sizeimage: 0,
                    colorspace: V4l2Colorspace::Unset,
                    private: 0,
	            },
	        },
	    };

	    if xioctl(fd, VIDIOC_S_FMT, (&mut fmt as *mut V4l2Format).cast()) == -1 {
		    panic!("Error setting Pixel Format\n");
	    }

	    // Request a video capture buffer.
	    let mut req = V4l2RequestBuffers {
	        count: 1,
	        type_: V4l2BufType::VideoCapture,
	        memory: V4l2Memory::Mmap,
	        reserved: [0; 2],
	    };

	     
	    if xioctl(fd, VIDIOC_REQBUFS, (&mut req as *mut V4l2RequestBuffers).cast()) == -1 {
		    panic!("Error Requesting Buffer\n");
	    }

	    // Query buffer
	    let mut buf = V4l2Buffer {
	        index: 0,
            type_: V4l2BufType::VideoCapture,
            bytesused: 0,
            flags: 0,
            field: V4l2Field::Any,
            timestamp: TimeVal {
                tv_sec: 0,
                tv_usec: 0,
            },
            timecode: V4l2Timecode {
                type_: 0,
                flags: 0,
                frames: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
                userbits: [0; 4],
            },
            sequence: 0,
            memory: V4l2Memory::Mmap,
            m: V4l2BufferUnion { userptr: 0 },
            length: 0,
            input: 0,
            reserved: 0,
	    };

	    if xioctl(fd, VIDIOC_QUERYBUF, (&mut buf as *mut V4l2Buffer).cast()) == -1 {
		    panic!("Error Querying Buffer\n");
	    }
        // FIXME: Raster
	    // unsafe { *output = mmap(null_mut(), buf.length.try_into().unwrap(), PROT_READ | PROT_WRITE, MAP_SHARED,
		//    fd, buf.m.offset.try_into().unwrap()) };

	    // Start the capture:
	    if xioctl(fd, VIDIOC_QBUF, (&mut buf as *mut V4l2Buffer).cast()) == -1 {
		    panic!("Error: VIDIOC_QBUF");
	    }

	    let mut type_ = V4l2BufType::VideoCapture;
	    if xioctl(fd, VIDIOC_STREAMON, (&mut type_ as *mut V4l2BufType).cast()) == -1 {
		    panic!("Error: VIDIOC_STREAMON");
	    }
	    
	    Some(Camera {
	        device: Device::new(fd, Watcher::new().input()),
	        size: buf.length,
	        buf,
	        buffer: null_mut(),
	        data: null_mut(),
	        raster,
	    })
    }
}

impl Future for Camera {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
	    if xioctl(self.device.fd(), VIDIOC_DQBUF, (&mut self.buf as *mut V4l2Buffer).cast()) == -1 {
	        let errno = errno();
		    if errno == /*EAGAIN*/11 {
		        self.device.register_waker(cx.waker());
		        return Poll::Pending;
	        }
	        unsafe {
    		    close(self.device.fd());
		    }
		    panic!("Error retrieving frame {}\n", errno);
	    }

	    if xioctl(self.device.fd(), VIDIOC_QBUF, (&mut self.buf as *mut V4l2Buffer).cast()) == -1 {
		    panic!("VIDIOC_QBUF");
	    }
	    
	    Poll::Ready(())
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
	    let mut type_ = V4l2BufType::VideoCapture;
	    if xioctl(self.device.fd(), VIDIOC_STREAMOFF, (&mut type_ as *mut V4l2BufType).cast()) == -1 {
		    panic!("Error VIDIOC_STREAMOFF");
	    }
	    if unsafe { munmap(self.buffer, self.size.try_into().unwrap()) == -1 } {
		    panic!("Error munmap");
	    }
	    if unsafe { close(self.device.fd()) == -1 }  {
		    panic!("Error close");
	    }
    }
}
