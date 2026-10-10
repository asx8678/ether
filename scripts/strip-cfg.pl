#!/usr/bin/perl
# strip-cfg.pl: remove Rust items gated by #[cfg(test)] and, with --windows, by
# Windows-only cfgs: #[cfg(windows)], #[cfg(target_os = "windows")], #[cfg(all(windows, ...))].
#
# Usage: strip-cfg.pl [--windows] --commit SHA --note-src SRC_PREFIX --note-dst DST_PREFIX FILE...
# Changed files are rewritten in place with a one-line "Modified from OpenAI Codex" note.
# Prints one TSV line per changed file: path, test items, windows items, flags.
# Flags mark extents worth a manual look (neg = closed by an outer brace, eof = ran to EOF).
use strict;
use warnings;
use open qw(:std :encoding(UTF-8));

my %opt = (windows => 0, commit => '', src => '', dst => '');
while (@ARGV && $ARGV[0] =~ /^--/) {
    my $o = shift @ARGV;
    if    ($o eq '--windows')  { $opt{windows} = 1 }
    elsif ($o eq '--commit')   { $opt{commit} = shift @ARGV }
    elsif ($o eq '--note-src') { $opt{src} = shift @ARGV }
    elsif ($o eq '--note-dst') { $opt{dst} = shift @ARGV }
    else                       { die "unknown option $o\n" }
}

my $TEST_RE = qr/^cfg\(\s*(?:test|all\(\s*test\b.*)\)$/s;
my $WIN_RE  = qr/^cfg\(\s*(?:windows|target_os\s*=\s*"windows"|all\(\s*(?:windows|target_os\s*=\s*"windows")\b.*)\)$/s;
my $ITEM_RE = qr/^(?:pub(?:\s*\([^)]*\))?\s+)?(?:(?:unsafe|async|const|default|extern(?:\s+"[^"]*")?)\s+)*(?:fn|mod|impl|struct|enum|trait|type|union|macro_rules!|static|const|use|extern\s+crate)\b/;

# Replace comments, string and char literals with spaces (newlines kept), so brace
# counting and attribute detection only see code.
sub code_mask {
    my ($s) = @_;
    my @c = split //, $s;
    my @m = @c;
    my $n = @c;
    my $i = 0;
    my $blank = sub { my ($a, $b) = @_; for my $k ($a .. $b) { $m[$k] = ' ' if $k < $n && $c[$k] ne "\n" } };
    while ($i < $n) {
        my $ch = $c[$i];
        if ($ch eq '/' && $i + 1 < $n && $c[$i + 1] eq '/') {
            my $k = $i;
            $k++ while $k < $n && $c[$k] ne "\n";
            $blank->($i, $k - 1);
            $i = $k;
            next;
        }
        if ($ch eq '/' && $i + 1 < $n && $c[$i + 1] eq '*') {
            my ($k, $d) = ($i, 0);
            while ($k < $n) {
                if ($c[$k] eq '/' && $k + 1 < $n && $c[$k + 1] eq '*') { $d++; $k += 2; next }
                if ($c[$k] eq '*' && $k + 1 < $n && $c[$k + 1] eq '/') { $d--; $k += 2; last if $d == 0; next }
                $k++;
            }
            $blank->($i, $k - 1);
            $i = $k;
            next;
        }
        if (($ch eq 'r' || ($ch eq 'b' && $i + 1 < $n && $c[$i + 1] eq 'r'))
            && ($i == 0 || $c[$i - 1] !~ /[A-Za-z0-9_]/)) {
            my $j = $i + ($ch eq 'b' ? 2 : 1);
            my $h = 0;
            while ($j < $n && $c[$j] eq '#') { $h++; $j++ }
            if ($j < $n && $c[$j] eq '"') {
                my $k = $j + 1;
                CLOSE: while ($k < $n) {
                    if ($c[$k] eq '"') {
                        my $ok = 1;
                        for my $t (1 .. $h) { if ($k + $t >= $n || $c[$k + $t] ne '#') { $ok = 0; last } }
                        last CLOSE if $ok;
                    }
                    $k++;
                }
                $blank->($i, $k + $h);
                $i = $k + $h + 1;
                next;
            }
        }
        if ($ch eq '"') {
            my $k = $i + 1;
            while ($k < $n) {
                if ($c[$k] eq '\\') { $k += 2; next }
                last if $c[$k] eq '"';
                $k++;
            }
            $blank->($i, $k);
            $i = $k + 1;
            next;
        }
        if ($ch eq "'") {
            if ($i + 1 < $n && $c[$i + 1] eq '\\') {
                my $k = $i + 3;
                $k++ while $k < $n && $c[$k] ne "'";
                $blank->($i, $k);
                $i = $k + 1;
                next;
            }
            if ($i + 2 < $n && $c[$i + 2] eq "'") { $blank->($i, $i + 2); $i += 3; next }
        }
        $i++;
    }
    return join('', @m);
}

# Last line of the item that starts after column $col of line $i.
sub extent {
    my ($M, $i, $col) = @_;
    my ($depth, $brace, $item, $attr) = (0, 0, undef, 0);
    for (my $j = $i; $j < @$M; $j++) {
        my $t = $j == $i ? substr($M->[$j], $col) : $M->[$j];
        (my $trim = $t) =~ s/^\s+|\s+$//g;
        next if $trim eq '' && $depth == 0;
        $attr = 1 if $depth == 0 && !$attr && $trim =~ /^#\[/;
        for my $ch (split //, $t) {
            if ($ch eq '{' || $ch eq '(' || $ch eq '[') { $depth++; $brace = 1 if $ch eq '{' && !$attr }
            elsif ($ch eq '}' || $ch eq ')' || $ch eq ']') {
                $depth--;
                return ($j - 1, 'neg') if $depth < 0;
            }
        }
        if ($attr) { $attr = 0 if $depth == 0; next }
        $item = ($trim =~ $ITEM_RE ? 1 : 0) unless defined $item;
        next unless $depth == 0;
        if ($item) { return ($j, '') if $trim =~ /;$/ || ($brace && $trim =~ /\}$/) }
        else       { return ($j, '') if $trim =~ /[;},]$/ }
    }
    return (scalar(@$M) - 1, 'eof');
}

for my $file (@ARGV) {
    open(my $fh, '<', $file) or die "$file: $!\n";
    my $src = do { local $/; <$fh> };
    close $fh;
    my @L = split /\n/, $src, -1;
    my @M = split /\n/, code_mask($src), -1;
    die "$file: mask line mismatch\n" unless @L == @M;
    my (@ranges, @flags);
    my ($nt, $nw) = (0, 0);
    for (my $i = 0; $i < @L; $i++) {
        push @flags, 'inner-cfg@' . ($i + 1) if $M[$i] =~ /^\s*#!\[/ && $L[$i] =~ /^\s*#!\[\s*cfg\(\s*(?:test|windows)/;
        next unless $M[$i] =~ /^\s*#\[/;
        next unless $L[$i] =~ /^(\s*#\[\s*(cfg\((?:[^\[\]"]|"[^"]*")*\))\s*\])/;
        my ($whole, $cfg) = ($1, $2);
        my $kind = $cfg =~ $TEST_RE ? 't' : ($opt{windows} && $cfg =~ $WIN_RE) ? 'w' : '';
        next unless $kind;
        my ($end, $flag) = extent(\@M, $i, length $whole);
        my $start = $i;
        while ($start > 0
            && (($L[$start - 1] =~ /^\s*#\[.*\]\s*$/ && $M[$start - 1] =~ /^\s*#\[/) || $L[$start - 1] =~ m{^\s*///})) {
            $start--;
        }
        push @ranges, [$start, $end];
        $kind eq 't' ? $nt++ : $nw++;
        push @flags, "$kind\@" . ($i + 1) . ":$flag" if $flag;
        $i = $end;
    }
    next unless @ranges;
    my %drop;
    for my $r (@ranges) { $drop{$_} = 1 for $r->[0] .. $r->[1] }
    my @out;
    for my $k (0 .. $#L) {
        next if $drop{$k};
        next if $L[$k] =~ /^\s*$/ && @out && $out[-1] =~ /^\s*$/;
        push @out, $L[$k];
    }
    pop @out while @out > 1 && $out[-1] eq '' && $out[-2] =~ /^\s*$/;
    my @what;
    push @what, 'removed unit tests'        if $nt;
    push @what, 'removed Windows-only code' if $nw;
    if (@out && $out[0] =~ m{^// Modified from OpenAI Codex \((.*)\): (.*)\.$}) {
        my ($where, $have) = ($1, $2);
        my @add = grep { index($have, $_) < 0 } @what;
        $out[0] = "// Modified from OpenAI Codex ($where): " . join('; ', $have, @add) . '.';
    } else {
        my $rel = $file;
        $rel =~ s/^\Q$opt{dst}\E/$opt{src}/ if $opt{dst} ne '';
        unshift @out, "// Modified from OpenAI Codex ($rel @ $opt{commit}): " . join('; ', @what) . '.';
    }
    open(my $oh, '>', $file) or die "$file: $!\n";
    print $oh join("\n", @out);
    close $oh;
    print join("\t", $file, $nt, $nw, join(',', @flags)), "\n";
}
