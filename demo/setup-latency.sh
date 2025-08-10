#!/bin/bash
set -e

REGION=$1

echo "Setting up network latency simulation for region: $REGION"

# Get default network interface (usually eth0 in containers)
INTERFACE=$(ip route | grep default | awk '{print $5}' | head -1)
echo "Using network interface: $INTERFACE"

# Clear any existing tc rules
tc qdisc del dev $INTERFACE root 2>/dev/null || true

case $REGION in
    "nyc")
        echo "Configuring NYC latencies:"
        echo "  → London: 80ms RTT (40ms delay)"
        echo "  → Tokyo: 150ms RTT (75ms delay)"
        
        # Create root qdisc
        tc qdisc add dev $INTERFACE root handle 1: htb default 30
        
        # Create classes for different destinations
        tc class add dev $INTERFACE parent 1: classid 1:1 htb rate 1gbit
        tc class add dev $INTERFACE parent 1:1 classid 1:10 htb rate 1gbit  # To London
        tc class add dev $INTERFACE parent 1:1 classid 1:20 htb rate 1gbit  # To Tokyo
        tc class add dev $INTERFACE parent 1:1 classid 1:30 htb rate 1gbit  # Default
        
        # Add latency to London (lis-london)
        tc qdisc add dev $INTERFACE parent 1:10 handle 10: netem delay 40ms 5ms distribution normal
        
        # Add latency to Tokyo (lis-tokyo)
        tc qdisc add dev $INTERFACE parent 1:20 handle 20: netem delay 75ms 8ms distribution normal
        
        # Create filters to route traffic to appropriate classes
        tc filter add dev $INTERFACE protocol ip parent 1:0 prio 1 handle 10 fw flowid 1:10
        tc filter add dev $INTERFACE protocol ip parent 1:0 prio 1 handle 20 fw flowid 1:20
        
        # Use iptables to mark packets (if available)
        which iptables >/dev/null 2>&1 && {
            iptables -t mangle -F 2>/dev/null || true
            iptables -t mangle -A OUTPUT -d lis-london -j MARK --set-mark 10 2>/dev/null || true
            iptables -t mangle -A OUTPUT -d lis-tokyo -j MARK --set-mark 20 2>/dev/null || true
        }
        ;;
        
    "london")
        echo "Configuring London latencies:"
        echo "  → NYC: 80ms RTT (40ms delay)"
        echo "  → Tokyo: 200ms RTT (100ms delay)"
        
        tc qdisc add dev $INTERFACE root handle 1: htb default 30
        tc class add dev $INTERFACE parent 1: classid 1:1 htb rate 1gbit
        tc class add dev $INTERFACE parent 1:1 classid 1:10 htb rate 1gbit  # To NYC
        tc class add dev $INTERFACE parent 1:1 classid 1:20 htb rate 1gbit  # To Tokyo
        tc class add dev $INTERFACE parent 1:1 classid 1:30 htb rate 1gbit  # Default
        
        tc qdisc add dev $INTERFACE parent 1:10 handle 10: netem delay 40ms 5ms distribution normal
        tc qdisc add dev $INTERFACE parent 1:20 handle 20: netem delay 100ms 10ms distribution normal
        
        tc filter add dev $INTERFACE protocol ip parent 1:0 prio 1 handle 10 fw flowid 1:10
        tc filter add dev $INTERFACE protocol ip parent 1:0 prio 1 handle 20 fw flowid 1:20
        
        which iptables >/dev/null 2>&1 && {
            iptables -t mangle -F 2>/dev/null || true
            iptables -t mangle -A OUTPUT -d lis-nyc -j MARK --set-mark 10 2>/dev/null || true
            iptables -t mangle -A OUTPUT -d lis-tokyo -j MARK --set-mark 20 2>/dev/null || true
        }
        ;;
        
    "tokyo")
        echo "Configuring Tokyo latencies:"
        echo "  → NYC: 150ms RTT (75ms delay)"  
        echo "  → London: 200ms RTT (100ms delay)"
        
        tc qdisc add dev $INTERFACE root handle 1: htb default 30
        tc class add dev $INTERFACE parent 1: classid 1:1 htb rate 1gbit
        tc class add dev $INTERFACE parent 1:1 classid 1:10 htb rate 1gbit  # To NYC
        tc class add dev $INTERFACE parent 1:1 classid 1:20 htb rate 1gbit  # To London
        tc class add dev $INTERFACE parent 1:1 classid 1:30 htb rate 1gbit  # Default
        
        tc qdisc add dev $INTERFACE parent 1:10 handle 10: netem delay 75ms 8ms distribution normal
        tc qdisc add dev $INTERFACE parent 1:20 handle 20: netem delay 100ms 10ms distribution normal
        
        tc filter add dev $INTERFACE protocol ip parent 1:0 prio 1 handle 10 fw flowid 1:10
        tc filter add dev $INTERFACE protocol ip parent 1:0 prio 1 handle 20 fw flowid 1:20
        
        which iptables >/dev/null 2>&1 && {
            iptables -t mangle -F 2>/dev/null || true
            iptables -t mangle -A OUTPUT -d lis-nyc -j MARK --set-mark 10 2>/dev/null || true
            iptables -t mangle -A OUTPUT -d lis-london -j MARK --set-mark 20 2>/dev/null || true
        }
        ;;
        
    *)
        echo "Unknown region: $REGION"
        echo "Valid regions: nyc, london, tokyo"
        exit 1
        ;;
esac

# Display current tc configuration
echo "Network latency configuration applied:"
tc qdisc show dev $INTERFACE

echo "Latency simulation ready for region: $REGION"