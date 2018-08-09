"""

FFmpeg Wrapper

"""

import subprocess
import os
import signal

from pydbsrt.tools.ffmpeg_wrapper import FFException

binary_path = None

signal.signal(signal.SIGPIPE, signal.SIG_DFL)

def initialize():
    """
    Looks for FFmpeg binary with OS environment
    :return:
    """
    global binary_path

    for path in os.environ["PATH"].split(os.pathsep):
        if os.path.exists(os.path.join(path, 'ffmpeg')):
            binary_path = os.path.join(path, 'ffmpeg')
    if not binary_path:
        raise FFException.BinaryNotFound("ffmpeg")


class FFmpeg(object):
    """
    FFmpeg Wrapper
    """

    def __init__(self):
        self.input_file = list()
        self.input_format = None

        self.output_file = None
        self.output_format = None

        self.hw_acceleration = None

        self.no_video = False
        self.no_audio = False

        self.video_decoder = None
        self.video_encoder = None
        self.video_filters = list()

        self.audio_decoder = None
        self.audio_encoder = None
        self.audio_filters = list()

        self.pixel_format = None

        self.safe = None

        self.built_cmd = None

    def set_safe(self, safe):
        """
        :arg safe(int)
        :param safe:
        :return:
        """
        self.safe = safe
        return self

    def add_input_file(self, infile):
        """
        Setter
        """
        self.input_file.append(infile)
        return self

    def set_input_format(self, fmt):
        """
        Setter
        """
        self.input_format = fmt
        return self

    def set_output_file(self, outfile):
        """
        Setter
        """
        self.output_file = outfile
        return self

    def set_output_format(self, fmt):
        """
        Setter
        """
        self.output_format = fmt
        return self

    def set_hardware_acceleration(self, hw_accel):
        """
        Setter
        """
        self.hw_acceleration = hw_accel
        return self

    # No Video
    def set_no_video(self, value=True):
        """
        No Video
        :return:
        """
        self.no_video = value;

    # No Audio
    def set_no_audio(self, value=True):
        """
        No Audio
        :return:
        """
        self.no_audio = value;

    # Video
    def set_video_decoder(self, video_decoder):
        """
        Setter
        """
        self.video_decoder = video_decoder
        return self

    def set_video_encoder(self, video_encoder):
        """
        Setter
        """
        self.video_encoder = video_encoder
        return self

    def add_video_filter(self, video_filter):
        """
        Setter
        """
        self.video_filters.append(video_filter)
        return self

    # Audio
    def set_audio_decoder(self, audio_decoder):
        """
        Setter
        """
        self.audio_decoder = audio_decoder
        return self

    def set_audio_encoder(self, audio_encoder):
        """
        Setter
        """
        self.audio_encoder = audio_encoder
        return self

    def add_audio_filter(self, audio_filter):
        """
        Setter
        """
        self.audio_filters.append(audio_filter)
        return self

    def set_pixel_format(self, pixel_format):
        """
        Setter
        :param pixel_format:
        :return:
        """
        self.pixel_format = pixel_format

    def build(self):
        """
        Builder
        """
        cmd = ['ffmpeg', '-y', '-hide_banner']

        # Hardware Acceleration
        if self.hw_acceleration:
            cmd += ['-hwaccel', self.hw_acceleration]

        # Safe
        if self.safe is not None:
            cmd += ['-safe', str(self.safe)]

        # Input
        if self.input_format:
            cmd += ['-f', self.input_format]
        if self.input_file:
            for infile in self.input_file:
                cmd += ['-i', infile]

        # Video Decoder
        if self.video_decoder:
            cmd += ['-c:v', self.video_decoder]

        # Audio Decoder
        if self.audio_decoder:
            cmd += ['-c:a', self.audio_decoder]

        # Video filters
        if self.video_filters:
            cmd += ['-vf', ','.join(vf.build() for vf in self.video_filters)]

        # Audio filters
        if self.audio_filters:
            cmd += ['-af', ','.join(af.build() for af in self.audio_filters)]

        # Video Encoder
        if self.video_encoder:
            cmd += ['-vcodec', self.video_encoder]

        # Audio Encoder
        if self.audio_encoder:
            cmd += ['-acodec', self.audio_encoder]

        if self.pixel_format:
            cmd += ['-pix_fmt', self.pixel_format]

        # Output
        if self.output_format:
            cmd += ['-f', self.output_format]
        if self.output_file:
            cmd += [self.output_file]
        else:
            cmd += ['-']

        self.built_cmd = cmd
        return self

    def run(self):
        """
        Runner
        """
        child_process = subprocess.Popen(self.built_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        return child_process

