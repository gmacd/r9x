use r9x_core::fdt::{DeviceTree, Range, RangeMapping, RegBlock, TranslatedReg};

static TEST1_DTB: &[u8] = include_bytes!("../lib/test/fdt/test1.dtb");

#[test]
fn find_by_path() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    // Find the first node.  Next token should not be the same node.
    let root = dt.find_by_path("/").unwrap();
    assert_eq!(dt.node_name(&root).unwrap(), "");

    // Misc lookups
    let soc = dt.find_by_path("/soc").unwrap();
    assert_eq!(dt.node_name(&soc).unwrap(), "soc");

    let eth = dt.find_by_path("/reserved-memory/linux,cma").unwrap();
    assert_eq!(dt.node_name(&eth).unwrap(), "linux,cma");

    assert_eq!(dt.find_by_path("/bar"), None);
    assert_eq!(dt.find_by_path("/reserved-memory/foo"), None);
}

#[test]
fn traverse_tree() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    let root = dt.root().unwrap();
    assert_eq!(dt.node_name(&root).unwrap(), "");
    assert_eq!(root.depth(), 0);

    let aliases = dt.children(&root).next().unwrap();
    assert_eq!(dt.node_name(&aliases).unwrap(), "aliases");
    assert_eq!(aliases.depth(), 1);

    let soc = dt.children(&root).nth(4).unwrap();
    assert_eq!(dt.node_name(&soc).unwrap(), "soc");
    assert_eq!(soc.depth(), 1);
    let uart = dt.children(&soc).nth(4).unwrap();
    assert_eq!(dt.node_name(&uart).unwrap(), "serial@7e201000");
    assert_eq!(uart.depth(), 2);

    let uart_parent = dt.parent(&uart).unwrap();
    assert_eq!(dt.node_name(&uart_parent).unwrap(), "soc");
    assert_eq!(uart_parent, soc);
}

#[test]
fn find_compatible() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    // Simple test for compatible where there's only a single match in the string list
    let mut dma_iter = dt.find_compatible("shared-dma-pool");
    let dma = dma_iter.next().unwrap();
    assert_eq!(dt.node_name(&dma).unwrap(), "linux,cma");
    assert_eq!(dma.depth(), 2);
    assert!(dma_iter.next().is_none());

    // First, then second matching compatible strings for the same element
    assert_eq!(
        dt.find_compatible("arm,pl011").flat_map(|n| dt.node_name(&n)).collect::<Vec<&str>>(),
        vec!["serial@7e201000"]
    );
    assert_eq!(
        dt.find_compatible("arm,primecell").flat_map(|n| dt.node_name(&n)).collect::<Vec<&str>>(),
        vec!["serial@7e201000"]
    );

    // Find multiple matching nodes
    assert_eq!(
        dt.find_compatible("brcm,bcm2835-sdhci")
            .flat_map(|n| dt.node_name(&n))
            .collect::<Vec<&str>>(),
        vec!["mmc@7e300000", "mmcnr@7e300000"]
    );

    // Doesn't find substrings
    assert!(
        dt.find_compatible("arm").flat_map(|n| dt.node_name(&n)).collect::<Vec<&str>>().is_empty()
    );

    // No match
    assert!(
        dt.find_compatible("xxxx").flat_map(|n| dt.node_name(&n)).collect::<Vec<&str>>().is_empty()
    );
}

#[test]
fn get_cells() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    let node = dt.find_by_path("/reserved-memory").unwrap();
    assert_eq!(
        dt.property(&node, "#address-cells").and_then(|p| dt.property_value_as_u32(&p)),
        Some(1)
    );
    assert_eq!(
        dt.property(&node, "#size-cells").and_then(|p| dt.property_value_as_u32(&p)),
        Some(1)
    );

    let node = dt.find_by_path("/soc/spi@7e204000").unwrap();
    assert_eq!(
        dt.property(&node, "#address-cells").and_then(|p| dt.property_value_as_u32(&p)),
        Some(1)
    );
    assert_eq!(
        dt.property(&node, "#size-cells").and_then(|p| dt.property_value_as_u32(&p)),
        Some(0)
    );
}

#[test]
fn iterate_over_children() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    let children = dt
        .find_by_path("/thermal-zones")
        .iter()
        .flat_map(|resmem| dt.children(resmem))
        .flat_map(|n| dt.node_name(&n))
        .collect::<Vec<&str>>();

    assert_eq!(children, vec!["cpu-thermal"]);
}

#[test]
fn iterate_over_device_types() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    let cpus = dt.find_device_type("cpu").flat_map(|n| dt.node_name(&n)).collect::<Vec<&str>>();
    assert_eq!(cpus, vec!["cpu@0", "cpu@1", "cpu@2", "cpu@3"]);
}

#[test]
fn get_reg() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    let uart = dt.find_by_path("/soc/serial@7e201000").unwrap();
    let uart_reg_raw = dt
        .property(&uart, "reg")
        .map(|p| dt.property_value_as_u32_iter(&p).collect::<Vec<u32>>())
        .unwrap();
    assert_eq!(uart_reg_raw, vec![0x7e20_1000, 0x200]);

    // Basic case - 1 addr and 1 length
    let uart_reg = dt.property_reg_iter(uart).collect::<Vec<RegBlock>>();
    assert_eq!(uart_reg, vec![RegBlock { addr: 0x7e20_1000, len: Some(0x200) }]);

    // Example with no length
    let spidev = dt.find_by_path("/soc/spi@7e204000/spidev@0").unwrap();
    let spidev_reg = dt.property_reg_iter(spidev).collect::<Vec<RegBlock>>();
    assert_eq!(spidev_reg, vec![RegBlock { addr: 0x0, len: None }]);

    // Example with > 1 reg
    let watchdog = dt.find_by_path("/soc/watchdog@7e100000").unwrap();
    let watchdog_reg = dt.property_reg_iter(watchdog).collect::<Vec<RegBlock>>();
    assert_eq!(
        watchdog_reg,
        vec![
            RegBlock { addr: 0x7e100000, len: Some(0x114) },
            RegBlock { addr: 0x7e00a000, len: Some(0x24) }
        ]
    );
}

#[test]
fn get_ranges() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    // Get raw reg
    let uart = dt.find_by_path("/soc/serial@7e201000").unwrap();
    let uart_reg = dt.property_reg_iter(uart).collect::<Vec<RegBlock>>();
    assert_eq!(uart_reg, vec![RegBlock { addr: 0x7e20_1000, len: Some(0x200) }]);

    // Get ranges for parent
    let soc = dt.parent(&uart).unwrap();
    let soc_ranges = dt.property_range_iter(soc).collect::<Vec<Range>>();
    assert_eq!(
        soc_ranges,
        vec![
            Range::Translated(RangeMapping {
                child_bus_addr: 0x7e000000,
                parent_bus_addr: 0x3f000000,
                len: 0x1000000
            }),
            Range::Translated(RangeMapping {
                child_bus_addr: 0x40000000,
                parent_bus_addr: 0x40000000,
                len: 0x1000
            }),
        ]
    );
}

/// A minimal hand-built DTB whose single property, "partial", has a 6-byte
/// value: one full u32 cell (1) plus two trailing bytes.  The padding after
/// the value is 0xAA, so an iterator that over-reads past the property's end
/// yields a recognisably garbage final cell instead of a silent zero.
///
/// Layout (all values big-endian):
///   0..40   header: magic, totalsize 96, off_dt_struct 40, off_dt_strings 84,
///             off_mem_rsvmap 40, version 2, last_comp 14, boot_cpuid 0,
///             size_dt_strings 12, size_dt_struct 44
///   40..48  BEGIN_NODE ""
///   48..60  PROP len 6 nameoff 1
///   60..68  value 00 00 00 01 00 00, pad AA AA
///   68..76  END_NODE ""
///   76..84  END
///   84..96  strings: "\0" "partial\0" (12 bytes incl. padding)
const PARTIAL_PROP_DTB: &[u8] = &[
    0xd0, 0x0d, 0xfe, 0xed, 0x00, 0x00, 0x00, 0x60, // magic, totalsize 96
    0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x54, // off_dt_struct 40, off_dt_strings 84
    0x00, 0x00, 0x00, 0x28, 0x00, 0x00, 0x00, 0x02, // off_mem_rsvmap 40, version 2
    0x00, 0x00, 0x00, 0x0e, 0x00, 0x00, 0x00, 0x00, // last_comp 14, boot_cpuid 0
    0x00, 0x00, 0x00, 0x0c, 0x00, 0x00, 0x00, 0x2c, // size_dt_strings 12, size_dt_struct 44
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, // BEGIN_NODE ""
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x06, // PROP len 6
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, //   nameoff 1, value cell 1
    0x00, 0x00, 0xaa, 0xaa, 0x00, 0x00, 0x00, 0x02, // value tail, pad AA AA, END_NODE
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09, //   name "", END token
    0x00, 0x00, 0x00, 0x00, 0x00, b'p', b'a', b'r', // strings: "\0" "partial\0"
    b't', b'i', b'a', b'l', 0x00, 0x00, 0x00, 0x00, //   + 3 padding bytes
];

#[test]
fn u32_iter_stops_at_property_end() {
    let dt = DeviceTree::new(PARTIAL_PROP_DTB).unwrap();
    let root = dt.root().unwrap();
    let prop = dt.property(&root, "partial").unwrap();

    // The trailing two bytes must not be read as a cell
    assert_eq!(dt.property_value_as_u32_iter(&prop).collect::<Vec<u32>>(), vec![1]);
}

#[test]
fn get_translated_reg() {
    let dt = DeviceTree::new(TEST1_DTB).unwrap();

    // Get translated reg, based on parent ranges
    let uart = dt.find_by_path("/soc/serial@7e201000").unwrap();
    let uart_reg = dt.property_translated_reg_iter(uart).collect::<Vec<TranslatedReg>>();
    assert_eq!(
        uart_reg,
        vec![TranslatedReg::Translated(RegBlock { addr: 0x3f20_1000, len: Some(0x200) })]
    );
}
