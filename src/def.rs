use std::mem;
use std::io;
use std::slice;
use time;

#[repr(C)]
pub struct PcapFileHeaderInFile {
    pub magic_num: u32, /* magic number */
    pub version_major : u16, /* major version number */
    pub version_minor : u16, /* minor version number */
    pub thiszone : i32, /* GMT to local correction */
    pub sigfigs : u32, /* accuracy of timestamps */
    pub snaplen : u32, /* max length of captured packets, in octets */
    pub network : u32, /* data link type */
}
const PCAP_VERSION_MAJOR : u16 = 2;
const PCAP_VERSION_MINOR : u16 = 4;
impl PcapFileHeaderInFile {
    pub fn new(snaplen: usize, linktype: u32) -> Option<PcapFileHeaderInFile> {
        let snap = snaplen as u32;
        if snap as usize != snaplen {
            None
        } else {
            Some(PcapFileHeaderInFile {
                magic_num: PcapMagic::NanoSecondResolution.into(),
                version_major: PCAP_VERSION_MAJOR,
                version_minor: PCAP_VERSION_MINOR,
                thiszone: 0,
                sigfigs: 0,
                snaplen: snap,
                network: linktype,
            })
        }
    }
}

pub struct PcapFileHeader {
    pub ns_res: bool,
    pub need_byte_swap: bool,
    pub network : u32,
    pub utc_offset : i32,
    pub snaplen : usize,
}
impl PcapFileHeader {
    fn new(header: PcapFileHeaderInFile) -> Option<Self> {
        let magic = if let Some(m) = PcapMagic::try_from(header.magic_num) { m } else { return None; };

        if magic.need_byte_swap() {
            header.version_major.swap_bytes();
            header.version_minor.swap_bytes();
            header.thiszone.swap_bytes();
            header.sigfigs.swap_bytes();
            header.snaplen.swap_bytes();
            header.network.swap_bytes();
        }

        let snaplen = header.snaplen as usize;
        if snaplen as u32 != header.snaplen {
            return None;
        }

        // docs say this version number hasn't changed since 1998, so this simplistic comparison
        // should suffice
        if header.version_major != PCAP_VERSION_MAJOR || header.version_minor != PCAP_VERSION_MINOR {
            return None;
        }

        return Some(PcapFileHeader {
            ns_res: magic.ns_res(),
            need_byte_swap: magic.need_byte_swap(),
            network: header.network,
            utc_offset: header.thiszone,
            snaplen: snaplen,
        });
    }
}


#[repr(u32)]
#[allow(dead_code)]
#[derive(Copy,Clone)]
enum PcapMagic {
    Normal = 0xa1b2c3d4,
    NanoSecondResolution = 0xa1b23c4d,
    ByteSwap = 0xd4c3b2a1,
    NanoSecondResolutionByteSwap = 0x4d3cb2a1,
}
impl PcapMagic {
    fn try_from(val: u32) -> Option<PcapMagic> {
        if val == PcapMagic::Normal as u32 {
            Some(PcapMagic::Normal)
        } else if val == PcapMagic::NanoSecondResolution as u32 {
            Some(PcapMagic::NanoSecondResolution)
        } else if val == PcapMagic::ByteSwap as u32 {
            Some(PcapMagic::ByteSwap)
        } else if val == PcapMagic::NanoSecondResolutionByteSwap as u32 {
            Some(PcapMagic::NanoSecondResolutionByteSwap)
        } else {
            None
        }
    }
    fn need_byte_swap(self) -> bool {
        match self {
            PcapMagic::Normal => false,
            PcapMagic::NanoSecondResolution => false,
            PcapMagic::ByteSwap => true,
            PcapMagic::NanoSecondResolutionByteSwap => true,
        }
    }
    fn ns_res(self) -> bool {
        match self {
            PcapMagic::Normal => false,
            PcapMagic::NanoSecondResolution => true,
            PcapMagic::ByteSwap => false,
            PcapMagic::NanoSecondResolutionByteSwap => true,
        }
    }
}
impl From<PcapMagic> for u32 {
    fn from(val: PcapMagic) -> u32 {
        val as u32
    }
}

#[repr(C)]
pub struct PcapRecordHeader {
    pub ts_sec : u32, /* timestamp seconds */
    pub ts_usec : u32, /* timestamp microseconds */
    pub incl_len : u32, /* number of octets of packet saved in file */
    pub orig_len : u32, /* actual length of packet */
}
impl PcapRecordHeader {
    pub fn swap_bytes(&mut self, file_header: &PcapFileHeader) {
        if file_header.need_byte_swap {
            self.ts_sec.swap_bytes();
            self.ts_usec.swap_bytes();
            self.incl_len.swap_bytes();
            self.orig_len.swap_bytes();
        }
    }

    pub fn get_time(&self, file_header: &PcapFileHeader) -> Option<time::Timespec> {
        let nsec = if file_header.ns_res { self.ts_usec } else { self.ts_usec * 1000 } as i32;
        let utc_off : i64 = file_header.utc_offset.into();
        let sec : i64 = self.ts_sec.into();
        if nsec >= 1_000_000_000 {
            None
        } else {
            Some(time::Timespec::new(sec + utc_off, nsec))
        }
    }
}

unsafe fn as_byte_slice_mut<'a, T>(src: &'a mut T) -> &'a mut [u8] {
    // TODO: this is likely undefined behaviour (creating two overlapping slices).
    let size = mem::size_of::<T>();
    let ptr : *mut T = src;

    let u8_ptr = ptr as *mut u8;
    return slice::from_raw_parts_mut(u8_ptr, size);
}
unsafe fn as_byte_slice<'a, T>(src: &'a T) -> &'a [u8] {
    // TODO: this is likely undefined behaviour (creating two overlapping slices).
    let size = mem::size_of::<T>();
    let ptr : *const T = src;

    let u8_ptr = ptr as *const u8;
    return slice::from_raw_parts(u8_ptr, size);
}
/// This function is only safe when invoked with a type for which every possible bit pattern is
/// valid. This is probably only true for structs with #[repr(C)] not containing any enums and not
/// requiring any padding. between members. You'll need to carefully analyze this manually.
unsafe fn read_type<T, R>(reader: &mut R) -> Result<T, io::Error>
        where T : Sized,
              R : io::Read {
    let mut val : T = mem::uninitialized();
    reader.read_exact(as_byte_slice_mut(&mut val))?;
    Ok(val)
}
pub fn read_file_header<R : io::Read>(reader: &mut R) -> Result<Option<PcapFileHeader>, io::Error> {
    unsafe { read_type(reader) }.map(PcapFileHeader::new)
}
pub fn read_record_header<R : io::Read>(reader: &mut R) -> Result<PcapRecordHeader, io::Error> {
    unsafe { read_type(reader) }
}
pub fn write_file_header<W: io::Write>(writer: &mut W, hdr: &PcapFileHeaderInFile) -> Result<(), io::Error> {
    writer.write_all(unsafe { as_byte_slice(hdr) })
}
pub fn write_record_header<W: io::Write>(writer: &mut W, hdr: &PcapRecordHeader) -> Result<(), io::Error> {
    writer.write_all(unsafe { as_byte_slice(hdr) })
}


/// Known identifiers for the types of packets that might be captured in a `pcap` file. This tells
/// you how to interpret the packets you receive.
/// 
/// Look at [tcpdump.org](http://www.tcpdump.org/linktypes.html) for the canonical list with
/// descriptions.
#[derive(Copy,Clone)]
#[repr(u32)]
#[allow(dead_code,non_camel_case_types)]
pub enum Linktype {
    NULL = 0,
    /// Ethernet packets
    ETHERNET = 1,
    AX25 = 3,
    IEEE802_5 = 6,
    ARCNET_BSD = 7,
    SLIP = 8,
    PPP = 9,
    FDDI = 10,
    PPP_HDLC = 50,
    PPP_ETHER = 51,
    ATM_RFC1483 = 100,
    /// IP packets (IPv4 or IPv6)
    RAW = 101,
    C_HDLC = 104,
    IEEE802_11 = 105,
    FRELAY = 107,
    LOOP = 108,
    LINUX_SLL = 113,
    LTALK = 114,
    PFLOG = 117,
    IEEE802_11_PRISM = 119,
    IP_OVER_FC = 122,
    SUNATM = 123,
    IEEE802_11_RADIOTAP = 127,
    ARCNET_LINUX = 129,
    APPLE_IP_OVER_IEEE1394 = 138,
    MTP2_WITH_PHDR = 139,
    MTP2 = 140,
    MTP3 = 141,
    SCCP = 142,
    DOCSIS = 143,
    LINUX_IRDA = 144,
    USER00_LINKTYPE = 147,
    USER01_LINKTYPE = 148,
    USER02_LINKTYPE = 149,
    USER03_LINKTYPE = 150,
    USER04_LINKTYPE = 151,
    USER05_LINKTYPE = 152,
    USER06_LINKTYPE = 153,
    USER07_LINKTYPE = 154,
    USER08_LINKTYPE = 155,
    USER09_LINKTYPE = 156,
    USER10_LINKTYPE = 157,
    USER11_LINKTYPE = 158,
    USER12_LINKTYPE = 159,
    USER13_LINKTYPE = 160,
    USER14_LINKTYPE = 161,
    USER15_LINKTYPE = 162,
    IEEE802_11_AVS = 163,
    BACNET_MS_TP = 165,
    PPP_PPPD = 166,
    GPRS_LLC = 169,
    GPF_T = 170,
    GPF_F = 171,
    LINUX_LAPD = 177,
    BLUETOOTH_HCI_H4 = 187,
    USB_LINUX = 189,
    PPI = 192,
    IEEE802_15_4 = 195,
    SITA = 196,
    ERF = 197,
    BLUETOOTH_HCI_H4_WITH_PHDR = 201,
    AX25_KISS = 202,
    LAPD = 203,
    PPP_WITH_DIR = 204,
    C_HDLC_WITH_DIR = 205,
    FRELAY_WITH_DIR = 206,
    IPMB_LINUX = 209,
    IEEE802_15_4_NONASK_PHY = 215,
    USB_LINUX_MMAPPED = 220,
    FC_2 = 224,
    FC_2_WITH_FRAME_DELIMS = 225,
    IPNET = 226,
    CAN_SOCKETCAN = 227,
    IPV4 = 228,
    IPV6 = 229,
    IEEE802_15_4_NOFCS = 230,
    DBUS = 231,
    DVB_CI = 235,
    MUX27010 = 236,
    STANAG_5066_D_PDU = 237,
    NFLOG = 239,
    NETANALYZER = 240,
    NETANALYZER_TRANSPARENT = 241,
    IPOIB = 242,
    MPEG_2_TS = 243,
    NG40 = 244,
    NFC_LLCP = 245,
    INFINIBAND = 247,
    SCTP = 248,
    USBPCAP = 249,
    RTAC_SERIAL = 250,
    BLUETOOTH_LE_LL = 251,
    NETLINK = 253,
    BLUETOOTH_LINUX_MONITOR = 254,
    BLUETOOTH_BREDR_BB = 255,
    BLUETOOTH_LE_LL_WITH_PHDR = 256,
    PROFIBUS_DL = 257,
    PKTAP = 258,
    EPON = 259,
    IPMI_HPM_2 = 260,
    ZWAVE_R1_R2 = 261,
    ZWAVE_R3 = 262,
    WATTSTOPPER_DLM = 263,
    ISO_14443 = 264,
    RDS = 265,
    USB_DARWIN = 266,
}
impl From<Linktype> for u32 {
    fn from(val: Linktype) -> u32 {
        val as u32
    }
}
