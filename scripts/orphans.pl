#!/usr/bin/perl
# orphans.pl: find .rs files in a crate that no crate root reaches through `mod`
# declarations (#[path] and include!("x.rs") are followed). With --delete, remove them
# and any directories left empty. Also reports `mod` declarations whose file is missing.
#
# Usage: orphans.pl CRATE_DIR [--delete]
# Output: "orphan <path>" and "missing <file>: mod <name>" lines.
use strict;
use warnings;
use File::Find;
use File::Basename qw(dirname basename);
use Cwd qw(abs_path);

my $dir    = shift @ARGV or die "usage: orphans.pl CRATE_DIR [--delete]\n";
my $delete = grep { $_ eq '--delete' } @ARGV;

my @roots;
for my $r ('src/lib.rs', 'src/main.rs', 'build.rs') { push @roots, "$dir/$r" if -f "$dir/$r" }
if (-d "$dir/src/bin") {
    opendir(my $dh, "$dir/src/bin") or die;
    for my $f (readdir $dh) {
        push @roots, "$dir/src/bin/$f"         if $f =~ /\.rs$/;
        push @roots, "$dir/src/bin/$f/main.rs" if -f "$dir/src/bin/$f/main.rs";
    }
    closedir $dh;
}
if (open(my $ct, '<', "$dir/Cargo.toml")) {
    while (<$ct>) { push @roots, "$dir/$1" if /^\s*path\s*=\s*"([^"]+\.rs)"/ && -f "$dir/$1" }
    close $ct;
}

my (%seen, @missing);
my @queue = map { [$_, 1] } @roots;
while (my $e = shift @queue) {
    my ($f, $modrs) = @$e;
    my $abs = abs_path($f) or next;
    next if $seen{$abs}++;
    open(my $fh, '<', $f) or next;
    my @lines = <$fh>;
    close $fh;
    my $fdir = dirname($f);
    my $base = $modrs ? $fdir : "$fdir/" . basename($f, '.rs');
    my $path_attr;
    for my $l (@lines) {
        if ($l =~ /^\s*#\[path\s*=\s*"([^"]+)"\]/) { $path_attr = $1; next }
        if ($l =~ /^\s*(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(?:r#)?([A-Za-z0-9_]+)\s*;/) {
            my $name = $1;
            if (defined $path_attr) {
                my $p = "$fdir/$path_attr";
                if (-f $p) { push @queue, [$p, 1] } else { push @missing, "$f: #[path = \"$path_attr\"] mod $name" }
            } elsif (-f "$base/$name.rs") {
                push @queue, ["$base/$name.rs", 0];
            } elsif (-f "$base/$name/mod.rs") {
                push @queue, ["$base/$name/mod.rs", 1];
            } else {
                push @missing, "$f: mod $name";
            }
            undef $path_attr;
            next;
        }
        if ($l =~ /include!\(\s*"([^"]+\.rs)"\s*\)/) { push @queue, ["$fdir/$1", 1] if -f "$fdir/$1" }
        undef $path_attr unless $l =~ /^\s*#\[/ || $l =~ /^\s*$/ || $l =~ m{^\s*//};
    }
}

my @orphans;
find({ no_chdir => 1, wanted => sub {
    return unless /\.rs$/ && -f $_;
    my $abs = abs_path($_);
    push @orphans, $File::Find::name unless $seen{$abs};
} }, grep { -d } "$dir/src");

print "orphan $_\n" for sort @orphans;
print "missing $_\n" for @missing;
if ($delete) {
    unlink @orphans;
    finddepth({ no_chdir => 1, wanted => sub { rmdir $_ if -d $_ } }, "$dir/src") if -d "$dir/src";
}
