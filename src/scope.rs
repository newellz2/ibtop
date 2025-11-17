use crate::services::lib::{Node, Port};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};

pub fn read_scope_file(path: &str) -> Vec<Node> {
    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    let mut nodes_map: HashMap<u64, Node> = HashMap::new();

    for (index, line) in reader.lines().enumerate() {
        let line = line.unwrap();

        // Skip the header line
        if index == 0 {
            continue;
        }

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split(',').collect();
        let parts_len = parts.len();
        if parts_len != 4 && parts_len != 5 {
            eprintln!("Warning: Skipping malformed line: {}", line);
            continue;
        }

        // Parse GUID - handle both hex (0x...) and decimal formats
        let guid_str = parts[0].trim();
        let guid = if guid_str.starts_with("0x") || guid_str.starts_with("0X") {
            // Parse as hexadecimal
            match u64::from_str_radix(&guid_str[2..], 16) {
                Ok(val) => val,
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to parse GUID '{}' as hex: {}. Skipping line.",
                        guid_str, e
                    );
                    continue;
                }
            }
        } else {
            // Parse as decimal
            match guid_str.parse::<u64>() {
                Ok(val) => val,
                Err(e) => {
                    eprintln!(
                        "Warning: Failed to parse GUID '{}' as decimal: {}. Skipping line.",
                        guid_str, e
                    );
                    continue;
                }
            }
        };

        let node_description = parts[1].trim().to_string();

        let lid = match parts[2].trim().parse::<u16>() {
            Ok(val) => val,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse LID '{}': {}. Skipping line.",
                    parts[2].trim(),
                    e
                );
                continue;
            }
        };

        let port_number = match parts[3].trim().parse::<i32>() {
            Ok(val) => val,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to parse port number '{}': {}. Skipping line.",
                    parts[3].trim(),
                    e
                );
                continue;
            }
        };

        let remote_node_description = if parts_len == 5 {
            parts[4].trim().to_string()
        } else {
            String::new()
        };

        // Create the port
        let port = Port {
            number: port_number,
            remote_node_description: remote_node_description,
        };

        // Add port to existing node or create new node
        nodes_map
            .entry(guid)
            .and_modify(|node| node.ports.push(port.clone()))
            .or_insert(Node {
                guid,
                node_description,
                lid,
                ports: vec![port],
            });
    }

    nodes_map.into_values().collect()
}
