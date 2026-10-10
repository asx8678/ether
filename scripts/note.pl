#!/usr/bin/perl
# note.pl FILE SOURCE WHAT: add or extend the first-line "Modified from OpenAI Codex" note in place.
# SOURCE is "codex-rs/<path> @ <sha>". Uses `#` for .toml files and `//` otherwise.
use strict;
use warnings;

my ($file, $src, $what) = @ARGV;
die "usage: note.pl FILE SOURCE WHAT\n" unless defined $what;
my $cm = $file =~ /\.toml$/ ? '#' : '//';
open(my $fh, '<', $file) or die "error: $file: $!\n";
my @lines = <$fh>;
close $fh;
if (@lines && $lines[0] =~ /^\Q$cm\E Modified from OpenAI Codex \((.*)\): (.*)\.\s*$/) {
    my ($where, $have) = ($1, $2);
    $lines[0] = "$cm Modified from OpenAI Codex ($where): $have; $what.\n" if index($have, $what) < 0;
} else {
    unshift @lines, "$cm Modified from OpenAI Codex ($src): $what.\n";
}
open(my $oh, '>', $file) or die "error: $file: $!\n";
print $oh @lines;
close $oh;
