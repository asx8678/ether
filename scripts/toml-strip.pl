#!/usr/bin/perl
# New in ether: strips test, bench and Windows sections from copied manifests.
# toml-strip.pl: remove test, bench and Windows-only sections from a crate's Cargo.toml
# in place: [dev-dependencies], [target.*.dev-dependencies], [[bench]], [[test]], [[example]],
# and [target.'cfg(windows)'.*] / [target.'cfg(target_os = "windows")'.*] tables.
# A changed file gets a one-line "Modified from OpenAI Codex" note.
#
# Usage: toml-strip.pl --commit SHA --note-src CODEX_PATH FILE
# Output: one TSV line per removed section: FILE, section, dependency names.
use strict;
use warnings;

my %opt = (commit => '', src => '');
while (@ARGV && $ARGV[0] =~ /^--/) {
    my $o = shift @ARGV;
    if    ($o eq '--commit')   { $opt{commit} = shift @ARGV }
    elsif ($o eq '--note-src') { $opt{src} = shift @ARGV }
    else                       { die "unknown option $o\n" }
}
my $file = shift @ARGV or die "usage: toml-strip.pl --commit SHA --note-src PATH FILE\n";
open(my $fh, '<', $file) or die "$file: $!\n";
my @lines = <$fh>;
close $fh;

my (@out, @removed, $skip, $cur);
for my $l (@lines) {
    if ($l =~ /^\s*(\[\[?)([^\]]+)\]\]?\s*$/) {
        my ($arr, $h) = ($1 eq '[[', $2);
        $skip = ($arr && $h =~ /^(?:bench|test|example)$/)
            || $h eq 'dev-dependencies'
            || $h =~ /^target\..*\.dev-dependencies$/
            || $h =~ /^target\.['"]cfg\((?:windows|target_os\s*=\s*\\?"windows\\?")\)['"]\./;
        if ($skip) { $cur = [$arr ? "[[$h]]" : "[$h]", []]; push @removed, $cur }
    } elsif ($skip && $l =~ /^([A-Za-z0-9_-]+)\s*(?:\.|=)/) {
        push @{ $cur->[1] }, $1;
    }
    push @out, $l unless $skip;
}
exit 0 unless @removed;

my @clean;
for my $l (@out) {
    next if $l =~ /^\s*$/ && @clean && $clean[-1] =~ /^\s*$/;
    push @clean, $l;
}
pop @clean while @clean && $clean[-1] =~ /^\s*$/;
my %what;
for my $r (@removed) {
    my $s = $r->[0];
    $what{ $s =~ /windows/ ? 'Windows-only dependencies' : $s =~ /dev-dependencies/ ? 'dev-dependencies' : 'test/bench targets' } = 1;
}
my $note = "# Modified from OpenAI Codex ($opt{src} @ $opt{commit}): removed " . join(', ', sort keys %what) . ".\n";
open(my $oh, '>', $file) or die "$file: $!\n";
print $oh $note, @clean;
close $oh;
print join("\t", $file, $_->[0], join(' ', @{ $_->[1] })), "\n" for @removed;
