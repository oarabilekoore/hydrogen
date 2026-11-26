package main

import "core:flags"
import "core:fmt"
import "core:os"

// Global_Args contains flags accepted before or after a command.
// Positional arguments are collected in overflow; the first is the command.
Global_Args :: struct {
	version:       bool `args:"name=version" usage:"Print Hydrogen version information."`,
	version_short: bool `args:"name=V,hidden" usage:"Short form of --version."`,
	overflow:      [dynamic]string,
}

print_usage :: proc() {
	fmt.println("Usage: hydrogen <command> [options]")
	fmt.println("")
	fmt.println("Commands:")
	fmt.println("  version     Print Hydrogen version information")
	fmt.println("")
	fmt.println("Global options:")
	fmt.println("  -V, --version   Print version information")
	fmt.println("  -h, --help      Show this help")
}

run_command :: proc(command: string) {
	switch command {
	case "version":
		fmt.printfln("Hydrogen %s (Odin rewrite, experimental)", HYDROGEN_VERSION)
	case "help":
		print_usage()
	case:
		fmt.printfln("hydrogen: unknown command '%s'", command)
		fmt.println("Run 'hydrogen --help' to see available commands.")
	}
}

main :: proc() {
	args := Global_Args{}

	parse_error := flags.parse(&args, os.args[1:], .Unix)
	if parse_error != nil {
		flags.print_errors(Global_Args, parse_error, os.args[0], .Unix)
		delete(args.overflow)
		return
	}

	if args.version || args.version_short {
		fmt.printfln("Hydrogen %s (Odin rewrite, experimental)", HYDROGEN_VERSION)
		delete(args.overflow)
		return
	}

	if len(args.overflow) == 0 {
		print_usage()
		delete(args.overflow)
		return
	}

	run_command(args.overflow[0])
	delete(args.overflow)
}
